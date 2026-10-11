//! Wall pieces (BB-81, docs/WALLS_SPEC.md §10): what a broken chunk
//! (`chunks.rs`) throws. Each piece is a few of the chunk's own sheet
//! pixels - a quarter brick with its mortar line, a plank splinter along
//! its grain - cut from where the chunk sat in the walls atlas, so a wall
//! comes apart into itself rather than into coloured squares.
//!
//! Presentation only, owned by `fx.rs`: it reads `Event::ChunksBroken`,
//! `Event::ChunksCharred` and `Event::EmberFlew`, uses `rand::rng()` and
//! is never snapshotted. Pieces fly with a fake height and a shadow,
//! bounce, lie where they landed for `piece_linger_seconds`, then settle
//! into the ground as the field's battle scars (`Scar`, BB-84): dimmed,
//! flat, kept for the round up to `fx_max_scars`. A charred chunk falls
//! as ash, its own pixels gone to charcoal; an ember arcs downwind as a
//! glowing 2 px block.

use std::collections::VecDeque;

use rand::RngExt;

use crate::canvas::{Canvas, Sheet};
use crate::chunks::{chunk_offset, origin, CHUNKS, CHUNK_PX};
use crate::math::{Color, Rectangle, Vec2};
use crate::obstacle::Material;
use crate::tuning::tuning;
use crate::{OBSTACLE_TEXTURE_SIZE, Position};

/// One flying or lying piece.
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pos: Position,
    vel: Vec2,
    /// Height over the ground, px; up is positive.
    z: f32,
    vz: f32,
    /// Its pixels in the walls atlas.
    src: Rectangle,
    /// A splinter turns end over end as it flies.
    tumble: bool,
    bounces: u8,
    age: f32,
    /// Seconds it has lain still.
    rest: f32,
    /// Multiplied into its pixels: white, or charcoal for ash.
    tint: Color,
}

/// A piece settled into the ground for the rest of the round.
#[derive(Clone, Copy, Debug)]
struct Scar {
    x: f32,
    y: f32,
    src: Rectangle,
    tint: Color,
}

/// An ember in flight (`Event::EmberFlew`): a glowing block on a low arc.
#[derive(Clone, Copy, Debug)]
struct Ember {
    from: Position,
    to: Position,
    age: f32,
}

/// What ash is, multiplied into a plank's own pixels.
const ASH: Color = Color { r: 0x48, g: 0x3c, b: 0x34, a: 255 };

impl Piece {
    fn lying(&self) -> bool {
        self.z <= 0.0 && self.vz == 0.0
    }
}

/// Every wall piece in the round.
#[derive(Default)]
pub struct Pieces {
    list: Vec<Piece>,
    scars: VecDeque<Scar>,
    embers: Vec<Ember>,
}

/// Which way a wood variant's grain runs (docs/WALLS_SPEC.md §5): planks
/// laid across split into horizontal strips, upright boards and logs into
/// vertical ones.
fn grain_across(variant: i32) -> bool {
    matches!(variant, 0 | 2)
}

impl Pieces {
    pub fn clear(&mut self) {
        self.list.clear();
        self.scars.clear();
        self.embers.clear();
    }

    /// Pieces settled into the ground so far.
    pub fn scars(&self) -> usize {
        self.scars.len()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The chunks in `broken` of the `material`/`variant` tile centred on
    /// `center` come apart: each splits into pieces that mostly leave along
    /// `dir` (the blow's way, zero for none), a third sprayed back off the
    /// struck face - or, when the tile `collapsed`, drop where they stood.
    pub fn throw(&mut self, material: Material, variant: i32, center: Position, broken: u16, dir: Vec2, collapsed: bool) {
        self.spill(material, variant, center, broken, dir, collapsed, Color::WHITE);
    }

    /// The chunks in `broken` of the burning plank wall centred on
    /// `center` charred through: they slump where they stood as ash.
    pub fn char_away(&mut self, center: Position, broken: u16) {
        self.spill(Material::Wood, 0, center, broken, Vec2::zero(), true, ASH);
    }

    /// An ember thrown from `from` that lands on `to`.
    pub fn ember(&mut self, from: Position, to: Position) {
        if self.embers.len() < 64 {
            self.embers.push(Ember { from, to, age: 0.0 });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spill(&mut self, material: Material, variant: i32, center: Position, broken: u16, dir: Vec2, collapsed: bool, tint: Color) {
        if !material.is_wall() || broken == 0 {
            return;
        }
        let t = tuning();
        let cap = t.fx_max_pieces.max(0) as usize;
        if cap == 0 {
            return;
        }
        let mut rng = rand::rng();
        let row = material.row_base() + variant;
        let o = origin(center);
        let wood = material == Material::Wood;
        let across = grain_across(variant);
        let base = crate::math::atan2(dir.y, dir.x);
        let aimed = dir.length() > 0.5;
        for i in (0..CHUNKS).filter(|i| broken & (1 << i) != 0) {
            let off = chunk_offset(i);
            for (px, py, w, h) in cuts(wood, across, &mut rng) {
                let (speed, angle) = if collapsed || !aimed {
                    (t.piece_speed * rng.random_range(0.0..0.35), rng.random_range(0.0..std::f32::consts::TAU))
                } else if rng.random_bool(0.35) {
                    // Spray off the struck face, back toward the gun.
                    (t.piece_speed * rng.random_range(0.2..0.7), base + std::f32::consts::PI + rng.random_range(-0.9..0.9))
                } else {
                    (t.piece_speed * rng.random_range(0.3..1.0), base + rng.random_range(-0.7..0.7))
                };
                let piece = Piece {
                    pos: Position::new(o.x + off.x + px, o.y + off.y + py),
                    vel: Vec2::new(crate::math::cos(angle) * speed, crate::math::sin(angle) * speed),
                    z: if collapsed { rng.random_range(2.0..12.0) } else { rng.random_range(2.0..10.0) },
                    vz: if collapsed { rng.random_range(0.0..60.0) } else { rng.random_range(50.0..170.0) },
                    src: Rectangle::new(off.x + px, row as f32 * OBSTACLE_TEXTURE_SIZE + off.y + py, w, h),
                    tumble: wood,
                    bounces: 0,
                    age: 0.0,
                    rest: 0.0,
                    tint,
                };
                if self.list.len() >= cap {
                    // The oldest lying piece makes room; failing that, the
                    // oldest of all.
                    let at = self.list.iter().position(Piece::lying).unwrap_or(0);
                    self.list.remove(at);
                }
                self.list.push(piece);
            }
        }
    }

    pub fn tick(&mut self, dt: f32) {
        let t = tuning();
        let (gravity, drag, bounce) = (t.debris_gravity, t.debris_air_drag, t.debris_bounce);
        let linger = t.piece_linger_seconds;
        let cap = t.fx_max_scars.max(0) as usize;
        let mut settled = Vec::new();
        self.list.retain_mut(|p| {
            p.age += dt;
            if p.lying() {
                p.rest += dt;
                if p.rest < linger {
                    return true;
                }
                settled.push(Scar { x: snap(p.pos.x), y: snap(p.pos.y), src: p.src, tint: scarred(p.tint, 1.0) });
                return false;
            }
            p.vz -= gravity * dt;
            p.z += p.vz * dt;
            if p.z <= 0.0 {
                p.z = 0.0;
                if p.vz < -60.0 && p.bounces < 3 {
                    p.vz = -p.vz * bounce;
                    p.vel = p.vel * 0.55;
                    p.bounces += 1;
                } else {
                    p.vz = 0.0;
                    p.vel = Vec2::zero();
                }
            }
            let k = crate::math::exp(-drag * dt);
            p.vel = p.vel * k;
            p.pos.x += p.vel.x * dt;
            p.pos.y += p.vel.y * dt;
            true
        });
        if cap > 0 {
            self.scars.extend(settled);
            while self.scars.len() > cap {
                self.scars.pop_front();
            }
        }
        let flight = t.ember_flight_seconds.max(0.05);
        self.embers.retain_mut(|e| {
            e.age += dt;
            e.age < flight
        });
    }

    /// Draw the pieces lying on the ground (`airborne` false: under
    /// everything that stands, over the scars) or the ones in the air with
    /// their shadows and the embers (`airborne` true: over the tiles and
    /// tanks). Whole 2 px blocks; a lying piece darkens into the ground
    /// over its last second, and stays there as a scar.
    pub fn draw(&self, c: &mut impl Canvas, airborne: bool) {
        let linger = tuning().piece_linger_seconds;
        if !airborne {
            for s in &self.scars {
                if c.culls(Position::new(s.x, s.y)) {
                    continue;
                }
                c.blit(Sheet::Walls, s.src, Rectangle::new(s.x, s.y, s.src.width, s.src.height), Vec2::zero(), 0.0, s.tint);
            }
        }
        for p in self.list.iter().filter(|p| p.lying() != airborne) {
            if c.culls(p.pos) {
                continue;
            }
            let (x, y) = (snap(p.pos.x), snap(p.pos.y));
            if airborne {
                let a = (60.0 * (1.0 - (p.z / 40.0).min(0.6))) as u8;
                c.fill_rect(x as i32 + 1, y as i32 + 1, p.src.width as i32, p.src.height as i32, Color::new(0x25, 0x25, 0x25, a));
                let lift = snap(p.z);
                // A splinter in flight turns end over end, a quarter at a
                // time.
                let turn = if p.tumble { ((p.age * 14.0) as i32 % 4) as f32 * 90.0 } else { 0.0 };
                let (w, h) = (p.src.width, p.src.height);
                let dest = Rectangle::new(x + w / 2.0, y - lift + h / 2.0, w, h);
                c.blit(Sheet::Walls, p.src, dest, Vec2::new(w / 2.0, h / 2.0), turn, p.tint);
            } else {
                // Its last second it sinks into the ground: the shadow
                // goes and it darkens to what it settles as.
                let sink = (1.0 - (linger - p.rest)).clamp(0.0, 1.0);
                let shadow = (70.0 * (1.0 - sink)) as u8;
                if shadow > 0 {
                    c.fill_rect(x as i32 + 1, y as i32 + 1, p.src.width as i32, p.src.height as i32, Color::new(0x25, 0x25, 0x25, shadow));
                }
                c.blit(Sheet::Walls, p.src, Rectangle::new(x, y, p.src.width, p.src.height), Vec2::zero(), 0.0, scarred(p.tint, sink));
            }
        }
        if airborne {
            let flight = tuning().ember_flight_seconds.max(0.05);
            for e in &self.embers {
                let k = e.age / flight;
                let at = Position::new(e.from.x + (e.to.x - e.from.x) * k, e.from.y + (e.to.y - e.from.y) * k);
                if c.culls(at) {
                    continue;
                }
                // A low arc, peaking a tile up halfway.
                let lift = 4.0 * k * (1.0 - k) * 28.0;
                let (x, y) = (snap(at.x), snap(at.y - lift));
                // Hot white-yellow on its way up, cooling to orange.
                let glow = if k < 0.5 { Color::new(0xFF, 0xE0, 0x6A, 255) } else { Color::new(0xF0, 0x7A, 0x22, 255) };
                c.fill_rect(x as i32, y as i32, 2, 2, glow);
                let (tx, ty) = (snap(at.x - (e.to.x - e.from.x) * 0.04), snap(at.y - lift + 2.0));
                c.fill_rect(tx as i32, ty as i32, 2, 2, Color::new(0xB8, 0x3A, 0x1A, 160));
            }
        }
    }
}

/// Snap to the 2 px block grid.
fn snap(v: f32) -> f32 {
    (v / 2.0).round() * 2.0
}

/// `tint` as far as `k` (0..1) toward what a scar settles to: dimmed by
/// `scar_shade`, flat on the ground.
fn scarred(tint: Color, k: f32) -> Color {
    let shade = tuning().scar_shade.clamp(0.0, 1.0);
    let f = 1.0 - (1.0 - shade) * k;
    let ch = |v: u8| (v as f32 * f) as u8;
    Color::new(ch(tint.r), ch(tint.g), ch(tint.b), tint.a)
}

/// How one 8 px chunk splits, as `(x, y, w, h)` inside it, whole 2 px
/// blocks: masonry into quarter-chunk blocks, some halved again; wood into
/// strips along its grain.
fn cuts(wood: bool, across: bool, rng: &mut impl rand::Rng) -> Vec<(f32, f32, f32, f32)> {
    let mut out = Vec::with_capacity(8);
    let s = CHUNK_PX;
    if wood {
        for k in 0..4 {
            let at = k as f32 * 2.0;
            let long = if rng.random_bool(0.5) { s } else { s - 2.0 };
            if across {
                out.push((0.0, at, long, 2.0));
            } else {
                out.push((at, 0.0, 2.0, long));
            }
        }
        return out;
    }
    let q = s / 2.0;
    for (qx, qy) in [(0.0, 0.0), (q, 0.0), (0.0, q), (q, q)] {
        match rng.random_range(0..10) {
            0..=3 => out.push((qx, qy, q, q)),
            4..=7 => {
                out.push((qx, qy, q, 2.0));
                out.push((qx, qy + 2.0, q, 2.0));
            }
            _ => {
                out.push((qx, qy, 2.0, 2.0));
                out.push((qx + 2.0, qy, 2.0, 2.0));
                out.push((qx, qy + 2.0, q, 2.0));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_splits_into_whole_blocks_inside_itself() {
        let mut rng = rand::rng();
        for wood in [false, true] {
            for _ in 0..50 {
                let area: f32 = cuts(wood, true, &mut rng)
                    .into_iter()
                    .inspect(|&(x, y, w, h)| {
                        assert!(x % 2.0 == 0.0 && y % 2.0 == 0.0 && w % 2.0 == 0.0 && h % 2.0 == 0.0);
                        assert!(x + w <= CHUNK_PX && y + h <= CHUNK_PX);
                    })
                    .map(|(_, _, w, h)| w * h)
                    .sum();
                assert!(area <= CHUNK_PX * CHUNK_PX);
            }
        }
    }

    #[test]
    fn pieces_fly_land_and_go() {
        let mut p = Pieces::default();
        p.throw(Material::Brick, 0, Position::new(64.0, 64.0), 0b11, Vec2::new(1.0, 0.0), false);
        assert!(!p.is_empty());
        let n = p.len();
        for _ in 0..120 {
            p.tick(1.0 / 60.0);
        }
        assert_eq!(p.len(), n, "two seconds in, every piece lies where it fell");
        assert!(p.list.iter().all(Piece::lying));
        for _ in 0..(60.0 * (tuning().piece_linger_seconds + 1.0)) as i32 {
            p.tick(1.0 / 60.0);
        }
        assert!(p.is_empty(), "nothing is left lying loose");
        assert_eq!(p.scars(), n, "every piece settled into the ground as a scar");
    }

    #[test]
    fn scars_keep_to_their_cap_oldest_first() {
        let mut p = Pieces::default();
        let cap = tuning().fx_max_scars as usize;
        p.list.push(Piece {
            pos: Position::new(-2.0, 0.0),
            vel: Vec2::zero(),
            z: 0.0,
            vz: 0.0,
            src: Rectangle::new(0.0, 0.0, 2.0, 2.0),
            tumble: false,
            bounces: 0,
            age: 0.0,
            rest: tuning().piece_linger_seconds,
            tint: Color::WHITE,
        });
        while p.scars.len() <= cap {
            let k = p.scars.len();
            p.scars.push_back(Scar { x: k as f32, y: 0.0, src: Rectangle::new(0.0, 0.0, 2.0, 2.0), tint: Color::WHITE });
        }
        p.tick(1.0 / 60.0);
        assert_eq!(p.scars(), cap);
        assert_eq!(p.scars.back().map(|s| s.x), Some(-2.0), "the newest is the piece that just settled");
    }

    #[test]
    fn ash_falls_where_the_chunk_stood_and_embers_burn_out() {
        let mut p = Pieces::default();
        p.char_away(Position::new(64.0, 64.0), 0b1);
        assert!(!p.is_empty());
        assert!(p.list.iter().all(|q| q.tint == ASH));
        p.ember(Position::new(0.0, 0.0), Position::new(64.0, 0.0));
        assert_eq!(p.embers.len(), 1);
        for _ in 0..(60.0 * (tuning().ember_flight_seconds + 0.1)) as i32 {
            p.tick(1.0 / 60.0);
        }
        assert!(p.embers.is_empty());
    }

    #[test]
    fn only_walls_throw_pieces() {
        let mut p = Pieces::default();
        p.throw(Material::Sandbag, 0, Position::new(64.0, 64.0), 0xFF, Vec2::zero(), true);
        assert!(p.is_empty());
    }
}
