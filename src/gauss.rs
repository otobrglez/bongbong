//! The gauss rail (docs/gauss-rail.md), its headless half: a slug's leg as
//! the picture keeps it (`RailSlug`, with what it went through,
//! `Pierce`), a charge's end as it is drawn (`ChargeEndFx`), the measures
//! the world half fires by (`damage`, `recoil_speed`), the module's cell,
//! and the composers in the effects language (docs/effects.md) - the
//! charge, the slug's white frame, its ionised trail and the bursts where it
//! went through things, the fizzle and the vent. The world half is
//! `simulation/gauss.rs`. Every choice is hashed from a slot, a position or
//! the clock, never rolled.

use serde::{Deserialize, Serialize};

use crate::math::{Color, Vec2};
use crate::obstacle::Material;
use crate::pyro::{self, Puff, RAIL, SHIELD, SMOKE, Shape};
use crate::shell::Owner;
use crate::tank::{ActiveWeapon, Charge, ChargeEnd, ChargeStage, Tank};
use crate::tuning::Tuning;
use crate::{OBSTACLE_GRID_SIZE, Position};

/// The charge's ring round the hull: its radius at the press and at full
/// (px).
pub const CHARGE_RING_PX: (f32, f32) = (34.0, 22.0);

/// The motes drawn in to the muzzle while a rail charges.
pub const CHARGE_MOTES: u32 = 10;

/// How long before a charge vents its ring flickers and the module steams
/// (s): the warning a hold is about to be lost.
pub const VENT_WARN_SECONDS: f32 = 0.6;

/// The white frame lights the field every this many px along its line at
/// night (`weather::lights_in`).
pub const FRAME_LIGHT_SPACING_PX: f32 = 96.0;

/// How far across its line the trail wobbles as it thins (px).
pub const TRAIL_WOBBLE_PX: f32 = 2.0;

/// How close to a slug's line the tall grass is laid flat (px).
pub const GRASS_REACH_PX: f32 = 12.0;

/// How long a burst where a slug went through something plays (s).
pub const PIERCE_SECONDS: f32 = 0.5;

/// How long the spark star where a slug stopped plays (s).
pub const STOP_SECONDS: f32 = 0.3;

/// How long a fizzle and a vent play (s).
pub const END_SECONDS: (f32, f32) = (0.15, 0.8);

/// What a slug went through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pierced {
    /// A tank's hull.
    Tank,
    /// A tank whose rainbow shield took it.
    Shield,
    Frog,
    /// A tile - a wall, a prop, a tree, a tower, a lamp post, the iron an
    /// overcharged slug cuts.
    Tile(Material),
}

/// One thing a slug went through and where it went in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pierce {
    pub at: Position,
    pub what: Pierced,
}

/// One leg of a slug as it is drawn (`Game::rail_slugs`): put on by
/// `Game::rail_show` - a round's, a replica's off `Event::RailSlug`, a
/// client's own release - and aged until its trail has thinned away.
#[derive(Clone, Debug, PartialEq)]
pub struct RailSlug {
    /// Where it is drawn from: the module's muzzle on the first leg, the
    /// exit on a later one.
    pub start: Position,
    /// Where it stopped, or went into a portal.
    pub end: Position,
    /// The leg ended going into a portal: no stop is drawn.
    pub portal: bool,
    pub overcharged: bool,
    /// What it went through, in order along it.
    pub pierces: Vec<Pierce>,
    /// Seconds since it was fired.
    pub age: f32,
    /// Hashed from its start: the drawing's variety.
    pub seed: u32,
}

impl RailSlug {
    pub fn new(start: Position, end: Position, portal: bool, overcharged: bool, pierces: Vec<Pierce>) -> RailSlug {
        RailSlug { start, end, portal, overcharged, pierces, age: 0.0, seed: crate::blast::seed_at(start, 0x6A_55) }
    }

    /// The unit vector it flies along.
    pub fn dir(&self) -> Vec2 {
        let d = self.end - self.start;
        let len = d.length();
        if len > f32::EPSILON { d * (1.0 / len) } else { Vec2::new(0.0, -1.0) }
    }

    /// Whether its picture is over.
    pub fn done(&self, t: &Tuning) -> bool {
        self.age >= t.gauss_trail_seconds.max(PIERCE_SECONDS)
    }
}

/// A charge's end as it is drawn (`Game::charge_ends`): the fizzle or the
/// vent at a module's muzzle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChargeEndFx {
    pub at: Position,
    pub end: ChargeEnd,
    pub age: f32,
    pub seed: u32,
}

impl ChargeEndFx {
    pub fn new(at: Position, end: ChargeEnd) -> ChargeEndFx {
        ChargeEndFx { at, end, age: 0.0, seed: crate::blast::seed_at(at, 0x4E_77) }
    }

    pub fn done(&self) -> bool {
        match self.end {
            ChargeEnd::Fizzled => self.age >= END_SECONDS.0,
            ChargeEnd::Vented => self.age >= END_SECONDS.1,
            ChargeEnd::Lapsed => true,
        }
    }
}

/// Where `tank`'s rail module's bore mouth is (`tank_art::RAIL_MUZZLE`).
pub fn muzzle(tank: &Tank) -> Position {
    tank.turret_point(crate::tank_art::RAIL_MUZZLE[tank.row.clamp(0, 11) as usize])
}

/// A slug's damage to the first tank or tower it goes through: a seat's
/// `gauss_damage`, an enemy's `gauss_enemy_damage`.
pub fn damage(owner: Owner, t: &Tuning) -> f32 {
    if owner.is_player() { t.gauss_damage } else { t.gauss_enemy_damage }
}

/// The recoil's speed for a chassis of mass factor `mass_factor` (px/s):
/// what a knock needs to slide `gauss_recoil_cells` cells (twice
/// `gauss_overcharge_recoil_factor` for an overcharged slug) on ground
/// whose skid friction is `friction` - dry ground's, so wet ground, a ford
/// and ice slide it further - over the mass factor to
/// `gauss_recoil_mass_exponent`.
pub fn recoil_speed(t: &Tuning, mass_factor: f32, overcharged: bool, friction: f32) -> f32 {
    let cells = t.gauss_recoil_cells * if overcharged { t.gauss_overcharge_recoil_factor } else { 1.0 };
    (2.0 * friction.max(0.0) * OBSTACLE_GRID_SIZE * cells).sqrt() / mass_factor.max(0.1).powf(t.gauss_recoil_mass_exponent)
}

/// Which of the rail module's seven cells (`tank_modules.png`,
/// `TANK_MODULE_GAUSS_COL`) `tank` shows at `time`: 6 the shot
/// (`Tank::rail_flash`); 5 at full, 5 and 6 alternating at 10 Hz
/// overcharged; that many charge cells lit, 1 to 4, while charging; else
/// 0 - idle, and a disabled tank's lights out.
pub fn module_cell(tank: &Tank, time: f32) -> i32 {
    if tank.rail_flash > 0.0 {
        return 6;
    }
    match tank.charge.filter(|c| c.weapon == ActiveWeapon::GaussRail) {
        Some(c) => match c.stage() {
            ChargeStage::Full => 5,
            ChargeStage::Overcharged => 5 + ((time * 10.0) as i32).rem_euclid(2),
            ChargeStage::Charging => 1 + ((c.progress() * 4.0) as i32).min(3),
        },
        None => 0,
    }
}

/// A jagged spark of two runs from `from` heading `angle` (radians), `len`
/// px in all: pale blue with a white head.
fn zigzag(out: &mut Vec<Shape>, from: Position, angle: f32, len: f32, seed: u32, k: u32) {
    let step = len / 2.0;
    let mut at = from;
    let mut a = angle;
    for i in 0..2 {
        a += (pyro::unit(seed, k * 16 + i) - 0.5) * 1.6;
        let to = Position::new(at.x + a.cos() * step, at.y + a.sin() * step);
        out.push(Shape::Line { from: at, to, width: 1.0, head: RAIL[3], tail: RAIL[3] });
        at = to;
    }
    out.push(Shape::Mark { pos: at, size: 2, color: RAIL[4] });
}

/// A charging rail (the glowing pass): a ring round the hull at `center`
/// closing from `CHARGE_RING_PX`'s first radius to its second and
/// dissolving in as the charge fills, `CHARGE_MOTES` motes drawn in to the
/// `muzzle`, and a glow on the muzzle growing with it. At full the ring
/// beats and the glow's core is white; overcharged the ring is white and
/// jumps a block either way, and sparks crackle off the rails; over its
/// last `VENT_WARN_SECONDS` before the vent the ring flickers and the
/// module steams. Pure in its inputs.
pub fn compose_charge(center: Position, muzzle: Position, charge: &Charge, seed: u32, time: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    let p = charge.progress();
    let stage = charge.stage();
    let tau = std::f32::consts::TAU;
    let (r0, r1) = CHARGE_RING_PX;
    let mut radius = r0 + (r1 - r0) * p;
    let mut cover = p.max(0.15);
    let mut color = RAIL[3];
    match stage {
        ChargeStage::Charging => {}
        ChargeStage::Full => {
            if ((time * 12.0) as i32).rem_euclid(2) == 1 {
                cover = 0.6;
            }
        }
        ChargeStage::Overcharged => {
            color = RAIL[4];
            let tick = (time * 20.0) as u32;
            radius += (pyro::unit(seed ^ tick.wrapping_mul(0x9E37_79B9), 7) - 0.5).signum() * pyro::BLOCK;
            let crackle = seed ^ ((time * 15.0) as u32).wrapping_mul(0xC2B2_AE35);
            for k in 0..2 {
                let a = pyro::unit(crackle, k) * tau;
                zigzag(&mut out, muzzle, a, 4.0 + 4.0 * pyro::unit(crackle, k + 8), crackle, k);
            }
        }
    }
    let vent_in = charge.vent_in();
    if vent_in < VENT_WARN_SECONDS {
        cover = if ((time * 20.0) as i32).rem_euclid(2) == 0 { 1.0 } else { 0.5 };
        let k = 1.0 - vent_in / VENT_WARN_SECONDS;
        for i in 0..2u32 {
            let rise = ((time * 1.5 + pyro::unit(seed, 40 + i)) % 1.0) * 10.0;
            let pos = Position::new(muzzle.x + (pyro::unit(seed, 50 + i) - 0.5) * 6.0, muzzle.y - rise);
            out.push(Shape::Puff(Puff {
                pos,
                radius: 2.0 + 2.0 * k,
                body: SMOKE[5],
                shadow: Some(SMOKE[4]),
                lit: Some(SMOKE[6]),
                core: None,
                cover: 0.5 + 0.5 * k,
            }));
        }
    }
    out.push(Shape::Arc { center, radius, width: pyro::BLOCK, from: 0.0, to: tau, color, cover });
    if stage == ChargeStage::Charging {
        for i in 0..CHARGE_MOTES {
            let a = pyro::unit(seed, i) * tau + time * 3.0;
            let d = 40.0 * (1.0 - ((time * 2.0 + pyro::unit(seed, i + 20)) % 1.0));
            let pos = Position::new(muzzle.x + a.cos() * d, muzzle.y + a.sin() * d);
            out.push(Shape::Mark { pos, size: 2, color: if d < 8.0 { RAIL[4] } else { RAIL[3] } });
        }
    }
    out.push(Shape::Glow { pos: muzzle, radius: 10.0 + 16.0 * p, color: pyro::alpha(RAIL[2], p) });
    if stage != ChargeStage::Charging {
        out.push(Shape::Glow { pos: muzzle, radius: 6.0, color: RAIL[4] });
    }
    out
}

/// A line of one-block marks from `from` to `to`, `offset` px across it,
/// the blocks kept where the field's Bayer pattern is under `cover`.
fn dithered_line(out: &mut Vec<Shape>, from: Position, to: Position, offset: f32, cover: f32, color: impl Fn(f32) -> Color, wobble: impl Fn(f32) -> f32) {
    let d = to - from;
    let len = d.length();
    if len < 1.0 || cover <= 0.0 {
        return;
    }
    let u = d * (1.0 / len);
    let across = Vec2::new(-u.y, u.x);
    let mut along = 0.0;
    while along <= len {
        let w = offset + wobble(along);
        let p = from + u * along + across * w;
        let (bx, by) = pyro::block_of(p.x, p.y);
        if pyro::bayer(bx, by) < cover {
            out.push(Shape::Mark { pos: p, size: 2, color: color(along) });
        }
        along += pyro::BLOCK;
    }
}

/// One leg of a slug, its glowing half (the glowing pass): for
/// `gauss_flash_seconds` the white frame - a band three blocks across in
/// white with pale blue one block out either side; then the ionised trail,
/// one block wide, pale blue while it is fresh and `BLUE_LT` after,
/// wobbling across its line as it thins and dissolving through the Bayer
/// pattern, with a sparse row of ions beside it; the sparks of each pierce
/// thrown out of its far side; the spark star and a fading glow where it
/// stopped. Pure in its inputs.
pub fn compose_slug(slug: &RailSlug, t: &Tuning) -> Vec<Shape> {
    let mut out = Vec::new();
    let dir = slug.dir();
    let across = Vec2::new(-dir.y, dir.x);
    let age = slug.age;
    let flash = t.gauss_flash_seconds.max(1e-3);
    if age < flash {
        for (w, color) in [(-2.0 * pyro::BLOCK, RAIL[3]), (2.0 * pyro::BLOCK, RAIL[3])] {
            dithered_line(&mut out, slug.start, slug.end, w, 1.0, |_| color, |_| 0.0);
        }
        for w in [-pyro::BLOCK, 0.0, pyro::BLOCK] {
            dithered_line(&mut out, slug.start, slug.end, w, 1.0, |_| RAIL[4], |_| 0.0);
        }
        let mid = slug.start + (slug.end - slug.start) * 0.5;
        out.push(Shape::Glow { pos: mid, radius: 48.0, color: pyro::alpha(RAIL[3], 0.6) });
    } else {
        let span = (t.gauss_trail_seconds - flash).max(1e-3);
        let a = (1.0 - (age - flash) / span).clamp(0.0, 1.0);
        if a > 0.0 {
            let cover = if a >= 0.5 { 1.0 } else { a * 2.0 };
            let color = if a > 0.6 { RAIL[3] } else { RAIL[1] };
            let phase = age * 6.0;
            let wobble = |along: f32| {
                let w = (along * 0.3 + phase).sin() * TRAIL_WOBBLE_PX * (1.0 - a);
                (w / pyro::BLOCK).round() * pyro::BLOCK
            };
            dithered_line(&mut out, slug.start, slug.end, 0.0, cover, |_| color, wobble);
            // The ions: a sparse row a block off the line, every third block.
            let len = (slug.end - slug.start).length();
            let mut along = 0.0;
            while along <= len {
                let p = slug.start + dir * along + across * (-2.0 * pyro::BLOCK);
                let (bx, by) = pyro::block_of(p.x, p.y);
                if pyro::bayer(bx, by) < a * 0.8 {
                    out.push(Shape::Mark { pos: p, size: 2, color: Color::new(0xF0, 0xF0, 0xF0, 255) });
                }
                along += 3.0 * pyro::BLOCK;
            }
        }
    }
    for (i, pierce) in slug.pierces.iter().enumerate() {
        pierce_sparks(&mut out, pierce, dir, age, slug.seed ^ (i as u32).wrapping_mul(0x85EB_CA6B));
    }
    if !slug.portal && age < STOP_SECONDS {
        let k = age / STOP_SECONDS;
        let back = dir * -1.0;
        for i in 0..6u32 {
            let a = back.y.atan2(back.x) + (pyro::unit(slug.seed, 60 + i) - 0.5) * 2.0;
            let reach = (6.0 + 14.0 * pyro::unit(slug.seed, 70 + i)) * pyro::ease_out(k);
            let tip = Position::new(slug.end.x + a.cos() * reach, slug.end.y + a.sin() * reach);
            out.push(Shape::Line { from: slug.end, to: tip, width: 1.0, head: RAIL[4], tail: RAIL[3] });
        }
        out.push(Shape::Glow { pos: slug.end, radius: 16.0, color: pyro::alpha(RAIL[2], 1.0 - k) });
    }
    out
}

/// The sparks one pierce throws out of its far side along `dir`.
fn pierce_sparks(out: &mut Vec<Shape>, pierce: &Pierce, dir: Vec2, age: f32, seed: u32) {
    if age >= PIERCE_SECONDS {
        return;
    }
    let k = age / PIERCE_SECONDS;
    let (count, colors): (u32, [Color; 2]) = match pierce.what {
        Pierced::Tank => (6, [RAIL[4], RAIL[3]]),
        Pierced::Shield => (6, [SHIELD[2], SHIELD[1]]),
        Pierced::Frog => (4, [RAIL[3], RAIL[3]]),
        Pierced::Tile(Material::Iron) => (6, [RAIL[4], Color::new(0xC1, 0xC1, 0xC1, 255)]),
        Pierced::Tile(_) => (3, [RAIL[4], RAIL[3]]),
    };
    let heading = dir.y.atan2(dir.x);
    for i in 0..count {
        let a = heading + (pyro::unit(seed, i) - 0.5) * 1.2;
        let reach = (8.0 + 18.0 * pyro::unit(seed, 10 + i)) * pyro::ease_out(k);
        let from = pierce.at + dir * (reach * 0.5);
        let tip = Position::new(pierce.at.x + a.cos() * reach, pierce.at.y + a.sin() * reach);
        if k < 0.7 {
            out.push(Shape::Line { from, to: tip, width: 1.0, head: colors[0], tail: colors[1] });
        }
    }
    if matches!(pierce.what, Pierced::Tank | Pierced::Shield) && k < 0.5 {
        let cover = 1.0 - k * 2.0;
        out.push(Shape::Arc {
            center: pierce.at,
            radius: 4.0 + 12.0 * k,
            width: pyro::BLOCK,
            from: 0.0,
            to: std::f32::consts::TAU,
            color: colors[0],
            cover,
        });
    }
}

/// One leg of a slug, its lit half (the lit pass): the chips a tile it went
/// through throws out of its far side, in the tile's own dust, falling as
/// they go, and two dark flecks of armour off a hull. Pure in its inputs.
pub fn compose_slug_lit(slug: &RailSlug) -> Vec<Shape> {
    let mut out = Vec::new();
    if slug.age >= PIERCE_SECONDS {
        return out;
    }
    let k = slug.age / PIERCE_SECONDS;
    let dir = slug.dir();
    let heading = dir.y.atan2(dir.x);
    for (n, pierce) in slug.pierces.iter().enumerate() {
        let seed = slug.seed ^ (n as u32).wrapping_mul(0x27D4_EB2F);
        let (count, ramp) = match pierce.what {
            Pierced::Tile(material) => match pyro::dust_of(material) {
                Some(ramp) => (6, ramp),
                None if material == Material::Glass => (6, [RAIL[3], Color::new(0xF0, 0xF0, 0xF0, 255), RAIL[4]]),
                None => continue,
            },
            Pierced::Tank => (2, [SMOKE[1], SMOKE[2], SMOKE[3]]),
            Pierced::Shield | Pierced::Frog => continue,
        };
        for i in 0..count {
            let a = heading + (pyro::unit(seed, i) - 0.5) * 1.2;
            let reach = (8.0 + 16.0 * pyro::unit(seed, 10 + i)) * pyro::ease_out(k);
            let fall = k * k * 10.0;
            let pos = Position::new(pierce.at.x + a.cos() * reach, pierce.at.y + a.sin() * reach + fall);
            let (bx, by) = pyro::block_of(pos.x, pos.y);
            if pyro::bayer(bx, by) < 1.0 - k * 0.6 {
                out.push(Shape::Mark { pos, size: 2, color: ramp[(i as usize) % 3] });
            }
        }
    }
    out
}

/// A charge's end at a module's muzzle (the glowing pass): a fizzle - four
/// `BLUE_LT` blocks dropping off the rails - or a vent - a plume of white
/// steam rising and leaning with the wind (`lean`, `pyro::smoke_lean`)
/// with three white sparks. A lapse draws nothing. Pure in its inputs.
pub fn compose_end(fx: &ChargeEndFx, lean: f32) -> Vec<Shape> {
    let mut out = Vec::new();
    match fx.end {
        ChargeEnd::Fizzled => {
            let k = (fx.age / END_SECONDS.0).clamp(0.0, 1.0);
            for i in 0..4u32 {
                let x = fx.at.x + (pyro::unit(fx.seed, i) - 0.5) * 8.0;
                let pos = Position::new(x, fx.at.y + k * k * 10.0);
                out.push(Shape::Mark { pos, size: 2, color: RAIL[1] });
            }
        }
        ChargeEnd::Vented => {
            let k = (fx.age / END_SECONDS.1).clamp(0.0, 1.0);
            for i in 0..5u32 {
                let lag = i as f32 * 0.12;
                let a = ((fx.age - lag) / END_SECONDS.1).clamp(0.0, 1.0);
                if a <= 0.0 {
                    continue;
                }
                let pos = Position::new(fx.at.x + lean * a * 12.0 + (pyro::unit(fx.seed, i) - 0.5) * 6.0, fx.at.y - a * 22.0);
                out.push(Shape::Puff(Puff {
                    pos,
                    radius: 3.0 + 4.0 * a,
                    body: SMOKE[5],
                    shadow: Some(SMOKE[4]),
                    lit: Some(SMOKE[6]),
                    core: None,
                    cover: 1.0 - a,
                }));
            }
            if k < 0.3 {
                for i in 0..3u32 {
                    let a = pyro::unit(fx.seed, 20 + i) * std::f32::consts::TAU;
                    let r = 4.0 + 10.0 * (k / 0.3);
                    out.push(Shape::Mark { pos: Position::new(fx.at.x + a.cos() * r, fx.at.y + a.sin() * r), size: 2, color: RAIL[4] });
                }
            }
        }
        ChargeEnd::Lapsed => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tuning::Tuning;

    fn on_grid(shapes: &[Shape]) -> bool {
        shapes.iter().all(|s| match *s {
            Shape::Mark { pos, .. } => pos.x.is_finite() && pos.y.is_finite(),
            _ => true,
        })
    }

    fn colours(shapes: &[Shape]) -> Vec<Color> {
        shapes
            .iter()
            .filter_map(|s| match *s {
                Shape::Mark { color, .. } | Shape::Arc { color, .. } => Some(color),
                Shape::Line { head, .. } => Some(head),
                _ => None,
            })
            .collect()
    }

    fn slug() -> RailSlug {
        RailSlug::new(
            Position::new(100.0, 200.0),
            Position::new(700.0, 200.0),
            false,
            false,
            vec![
                Pierce { at: Position::new(300.0, 200.0), what: Pierced::Tile(Material::Brick) },
                Pierce { at: Position::new(500.0, 200.0), what: Pierced::Tank },
            ],
        )
    }

    #[test]
    fn the_recoil_speed_slides_the_cells_asked() {
        let t = Tuning::DEFAULT;
        let friction = crate::sonic::skid_friction(&t, 1.0);
        for (m, want) in [(1.0, 32.0), (216.0 / 280.0, 32.0 / (216.0 / 280.0)), (528.0 / 280.0, 32.0 / (528.0 / 280.0))] {
            let v = recoil_speed(&t, m, false, friction);
            let slide = crate::sonic::slide(&t, v, 1.0);
            assert!((slide - want).abs() < 0.5, "mass {m}: {slide} px, wanted {want}");
        }
        let v = recoil_speed(&t, 1.0, true, friction);
        assert!((crate::sonic::slide(&t, v, 1.0) - 64.0).abs() < 0.5, "overcharged slides twice as far");
    }

    #[test]
    fn the_charge_is_in_its_ramp_and_pure() {
        let charge = Charge { weapon: ActiveWeapon::GaussRail, held: 0.5 };
        let a = compose_charge(Position::new(200.0, 200.0), Position::new(190.0, 180.0), &charge, 7, 1.25);
        let b = compose_charge(Position::new(200.0, 200.0), Position::new(190.0, 180.0), &charge, 7, 1.25);
        assert_eq!(a, b, "the same inputs, the same picture");
        assert!(on_grid(&a));
        assert!(colours(&a).iter().all(|c| RAIL.contains(c)), "a charge is drawn in the rail's ramp");
        assert!(a.iter().any(|s| matches!(s, Shape::Arc { .. })), "a ring");
        assert!(a.iter().filter(|s| matches!(s, Shape::Mark { .. })).count() >= CHARGE_MOTES as usize, "the motes");
    }

    #[test]
    fn the_slug_is_in_its_ramp_and_gone_by_its_end() {
        let t = Tuning::DEFAULT;
        let mut s = slug();
        let frame = compose_slug(&s, &t);
        assert!(frame.iter().any(|x| matches!(x, Shape::Mark { color, .. } if *color == RAIL[4])), "the white frame");
        s.age = t.gauss_flash_seconds + 0.1;
        let trail = compose_slug(&s, &t);
        assert!(!trail.is_empty());
        let allowed = |c: &Color| RAIL.contains(c) || *c == Color::new(0xF0, 0xF0, 0xF0, 255) || *c == Color::new(0xC1, 0xC1, 0xC1, 255);
        assert!(colours(&trail).iter().all(allowed), "the trail and its sparks are in the rail's ramp");
        s.age = t.gauss_trail_seconds;
        assert!(compose_slug(&s, &t).iter().all(|x| matches!(x, Shape::Glow { .. })) || compose_slug(&s, &t).is_empty());
        assert!(s.done(&t));
    }

    #[test]
    fn the_white_frame_lasts_its_frames() {
        let t = Tuning::DEFAULT;
        let mut s = slug();
        let white = |shapes: &[Shape]| shapes.iter().filter(|x| matches!(x, Shape::Mark { color, .. } if *color == RAIL[4])).count();
        assert!(white(&compose_slug(&s, &t)) > 300, "three blocks across the whole leg");
        s.age = t.gauss_flash_seconds;
        assert!(white(&compose_slug(&s, &t)) < 20, "gone after its frames but a spark or two");
    }

    #[test]
    fn the_trail_dissolves_through_the_bayer_pattern() {
        let t = Tuning::DEFAULT;
        let mut s = slug();
        s.pierces.clear();
        s.portal = true;
        let count = |age: f32| {
            let mut s = s.clone();
            s.age = age;
            compose_slug(&s, &t).iter().filter(|x| matches!(x, Shape::Mark { color, .. } if *color == RAIL[1] || *color == RAIL[3])).count()
        };
        let span = t.gauss_trail_seconds - t.gauss_flash_seconds;
        let fresh = count(t.gauss_flash_seconds + 0.01);
        let late = count(t.gauss_flash_seconds + span * 0.85);
        assert!(late < fresh / 2, "thinned: {late} of {fresh}");
        s.age = 0.0;
    }

    #[test]
    fn a_pierce_burst_throws_out_of_the_far_side() {
        let mut s = slug();
        s.age = PIERCE_SECONDS * 0.5;
        let lit = compose_slug_lit(&s);
        let brick = s.pierces[0].at;
        let chips: Vec<_> = lit
            .iter()
            .filter_map(|x| match *x {
                Shape::Mark { pos, .. } if (pos.x - brick.x).abs() < 30.0 => Some(pos),
                _ => None,
            })
            .collect();
        assert!(!chips.is_empty(), "the brick throws chips");
        assert!(chips.iter().all(|p| p.x >= brick.x), "out of its far side, along the slug: {chips:?}");
        s.age = PIERCE_SECONDS;
        assert!(compose_slug_lit(&s).is_empty(), "gone by its end");
    }

    #[test]
    fn a_charge_end_is_gone_by_its_end() {
        for end in [ChargeEnd::Fizzled, ChargeEnd::Vented] {
            let mut fx = ChargeEndFx::new(Position::new(50.0, 50.0), end);
            assert!(!compose_end(&fx, 0.0).is_empty(), "{end:?} draws something");
            fx.age = END_SECONDS.1;
            assert!(fx.done(), "{end:?} is over");
        }
        assert!(compose_end(&ChargeEndFx::new(Position::new(0.0, 0.0), ChargeEnd::Lapsed), 0.0).is_empty());
    }

    #[test]
    fn the_module_shows_the_charge() {
        let mut tank = Tank { gauss_slugs: 2, ..Default::default() };
        assert_eq!(module_cell(&tank, 0.0), 0);
        tank.charge = Some(Charge { weapon: ActiveWeapon::GaussRail, held: 0.0 });
        assert_eq!(module_cell(&tank, 0.0), 1);
        tank.charge = Some(Charge { weapon: ActiveWeapon::GaussRail, held: 1.2 });
        assert_eq!(module_cell(&tank, 0.0), 4);
        tank.charge = Some(Charge { weapon: ActiveWeapon::GaussRail, held: 1.5 });
        assert_eq!(module_cell(&tank, 0.0), 5);
        tank.rail_flash = 0.1;
        assert_eq!(module_cell(&tank, 0.0), 6);
    }
}
