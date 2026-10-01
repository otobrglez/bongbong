//! Every hit's burst and every muzzle's flash, in the effects language
//! (`pyro.rs`, docs/effects.md): composed at draw time from blocks and
//! ramp steps, so a shell striking a wall is the same fire as the blast of
//! a drum, only smaller, and a plasma bolt's ring is drawn like the
//! tesla's.
//!
//! - A **shell** throws a small fireball back off whatever it struck - a
//!   white-hot flash with short rays, a few puffs burning out into smoke
//!   that climbs and leans with the wind, hot fragments flung back toward
//!   the gun.
//! - A **bullet** is a spark star and a wisp.
//! - A **plasma** bolt is a ring of energy racing out in its own colour,
//!   teal crackling with forks of lightning, purple turning spiral arms.
//! - A **laser** leaves a molten spot: a core in the beam's colour and
//!   droplets of slag splashing back.
//! - A **tesla** strike is a violet ring and jagged bolts; a bio slush
//!   glob a splash of ooze.
//! - A **ricochet** is a spark star; a **deflection** off a rainbow shield
//!   the same in the shield's violet.
//!
//! `compose` is a pure function of the hit, its age, its seed (a hash of
//! where it landed) and the wind, so a replica, a paused frame and a test
//! all draw the same burst; the particle layer adds its sparks and chips.

use crate::fx::ImpactKind;
use crate::math::{Color, Vec2};
use crate::plasma::PlasmaVariant;
use crate::pyro::{self, ease_out, fire_puff, unit, Puff, Shape, BLOCK, CHAR, FIRE, LASER_BLUE, LASER_RED, OOZE, PLASMA_PURPLE, PLASMA_TEAL, SHIELD, SMOKE, TESLA};
use crate::tuning::tuning;
use crate::Position;
use std::f32::consts::{PI, TAU};

/// Everything to paint for a hit of `kind` at `pos`, `age` seconds in,
/// the shot having travelled along `dir` (a unit vector), `lean` how far
/// its smoke drifts sideways per px it climbs (`pyro::smoke_lean`).
pub fn compose(kind: ImpactKind, pos: Position, dir: Vec2, age: f32, seed: u32, lean: f32) -> Vec<Shape> {
    let life = kind.seconds().max(0.01);
    let scale = tuning().hit_fx_scale;
    let t = (age / life).clamp(0.0, 1.0);
    let mut out = Vec::new();
    match kind {
        ImpactKind::Shell => shell(&mut out, pos, dir, age, life, seed, lean, scale),
        ImpactKind::Bullet => star(&mut out, pos, age, seed, scale, &[FIRE[7], FIRE[6], FIRE[5], FIRE[3]], true),
        ImpactKind::Ricochet => star(&mut out, pos, age, seed, scale * 0.8, &[FIRE[7], FIRE[6], FIRE[5], FIRE[3]], false),
        ImpactKind::Deflect => star(&mut out, pos, age, seed, scale * 0.9, &[SHIELD[2], SHIELD[2], SHIELD[1], SHIELD[0]], false),
        ImpactKind::Plasma(variant) => {
            let ramp = match variant {
                PlasmaVariant::Teal => &PLASMA_TEAL,
                PlasmaVariant::Purple => &PLASMA_PURPLE,
            };
            ring(&mut out, pos, t, seed, scale, ramp, variant == PlasmaVariant::Purple);
        }
        ImpactKind::Tesla => ring(&mut out, pos, t, seed, scale * 0.85, &TESLA, false),
        ImpactKind::Laser(blue) => laser(&mut out, pos, dir, age, t, seed, scale, if blue { &LASER_BLUE } else { &LASER_RED }),
        ImpactKind::Ooze => ooze(&mut out, pos, age, t, seed, scale),
        ImpactKind::Dust(material) => {
            if let Some(ramp) = pyro::dust_of(material) {
                dust(&mut out, pos, dir, age, life, seed, lean, ramp, false);
            }
        }
        ImpactKind::Collapse(material) => {
            if let Some(ramp) = pyro::dust_of(material) {
                dust(&mut out, pos, dir, age, life, seed, lean, ramp, true);
            }
        }
        ImpactKind::Ash => dust(&mut out, pos, dir, age, life, seed, lean, [SMOKE[1], SMOKE[2], SMOKE[3]], true),
    }
    out
}

/// Dust knocked off a tile - stone, sand, sawdust or leaves, in the
/// material's own shadow, body and lit steps (`pyro::dust_of`): a few
/// puffs thrown back off the face that was hit, rising a little, leaning
/// with the wind and thinning away. `collapse` is a whole tile coming
/// down: a bigger cloud rolling out all round where it stood, low and
/// wide, climbing as it settles.
#[allow(clippy::too_many_arguments)]
fn dust(out: &mut Vec<Shape>, pos: Position, dir: Vec2, age: f32, life: f32, seed: u32, lean: f32, ramp: [Color; 3], collapse: bool) {
    let u = |k: u32| unit(seed, k);
    // Thrown back toward the gun, or up off the face when the shot's way
    // is not known.
    let back = if dir.length() > 0.01 { Vec2::new(-dir.x, -dir.y) } else { Vec2::new(0.0, -1.0) };
    let (n, r0, reach, rise, spread) = if collapse {
        (9 + (u(1) * 4.0) as u32, 9.0, 20.0, 14.0, 0.0)
    } else {
        (3 + (u(1) * 3.0) as u32, 5.0, 8.0, 7.0, 2.2)
    };
    let mut puffs: Vec<Puff> = Vec::new();
    for i in 0..n {
        let s = 10 + i * 5;
        let born = if collapse { 0.12 * u(s) } else { 0.04 * u(s) };
        let tau = age - born;
        if tau <= 0.0 {
            continue;
        }
        // Each puff's own share of the life left when it was thrown, so
        // they all settle by the end.
        let q = (tau / (life - born).max(0.01)).clamp(0.0, 1.0);
        let grow = ease_out(tau / if collapse { 0.3 } else { 0.15 });
        let way = if collapse {
            let a = TAU * (i as f32 + 0.5 * u(s + 1)) / n as f32;
            // Rolled out wide and low, the field seen from above.
            Vec2::new(a.cos(), a.sin() * 0.6)
        } else {
            turn(back, (u(s + 1) - 0.5) * spread)
        };
        let d = reach * (0.35 + 0.65 * u(s + 2)) * grow;
        let climb = rise * (0.3 + 0.7 * u(s + 3)) * ease_out(q);
        let at = Position::new(pos.x + way.x * d + lean * climb, pos.y + way.y * d - climb);
        // Settles from halfway: the puffs shrink away, thinning only at
        // the very end.
        let settle = ((q - 0.5) / 0.5).clamp(0.0, 1.0);
        let r = r0 * (0.6 + 0.5 * u(s + 4)) * (0.5 + 0.5 * grow) * (1.0 + 0.6 * q) * (1.0 - 0.7 * settle);
        let cover = 1.0 - settle * settle;
        if r < BLOCK || cover <= 0.0 {
            continue;
        }
        puffs.push(Puff { pos: at, radius: r, body: ramp[1], shadow: Some(ramp[0]), lit: Some(ramp[2]), core: None, cover });
    }
    // Back to front: the puffs further up the field behind the nearer.
    puffs.sort_by(|a, b| a.pos.y.total_cmp(&b.pos.y));
    out.extend(puffs.into_iter().map(Shape::Puff));
}

/// The unit vector `a` radians off `dir`.
fn turn(dir: Vec2, a: f32) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(dir.x * c - dir.y * s, dir.x * s + dir.y * c)
}

/// A shell's burst: a flash, a small fireball splashing back off what it
/// struck and cooling into smoke, fragments flung back toward the gun.
#[allow(clippy::too_many_arguments)]
fn shell(out: &mut Vec<Shape>, pos: Position, dir: Vec2, age: f32, life: f32, seed: u32, lean: f32, scale: f32) {
    let u = |k: u32| unit(seed, k);
    let back = Vec2::new(-dir.x, -dir.y);
    let r0 = 10.0 * scale;
    let mut glows = Vec::new();

    // The fireball: puffs splashed back off the surface, each burning out
    // on its own clock into smoke that climbs, leans with the wind and
    // thins away.
    let n = 6 + (u(1) * 3.0) as u32;
    let mut puffs: Vec<(f32, Puff)> = Vec::new();
    // The fire's light: one glow over the hottest puff, not one per puff,
    // which add up to a white blot where two shells land together.
    let mut hottest: Option<(f32, Position)> = None;
    for i in 0..n {
        let s = 10 + i * 5;
        let born = 0.025 * u(s);
        let tau = age - born;
        if tau <= 0.0 {
            continue;
        }
        let q = tau / life;
        let a = (u(s + 1) - 0.5) * 2.4;
        let way = turn(back, a);
        let grow = ease_out(tau / 0.08);
        let out_k = u(s + 2);
        let reach = r0 * (0.3 + 0.9 * out_k) * grow;
        // Climbs slowly while it burns, then faster as smoke.
        let climb = r0 * (0.3 * q + 2.0 * q * q);
        let at = Position::new(pos.x + way.x * reach + lean * climb, pos.y + way.y * reach - climb);
        // The puffs nearest the strike burn longest, the fire holding its
        // gold for a fifth of a second before it reddens into smoke.
        let hot = life * (0.4 + 0.25 * u(s + 3)) * (1.25 - 0.45 * out_k);
        let heat = (1.0 - tau / hot).clamp(0.0, 1.0).powf(0.85);
        // The smoke breaks up as it goes: shrinking, and thinning only
        // at the very end.
        let thin = ((q - 0.55) / 0.45).clamp(0.0, 1.0);
        let r = r0 * (0.45 + 0.35 * u(s + 4)) * (0.5 + 0.5 * grow) * (1.0 + 0.6 * q) * (1.0 - 0.65 * thin);
        if r < BLOCK || thin >= 1.0 {
            continue;
        }
        // Cordite smoke: a mid grey paling as it climbs, never soot, and
        // clean of fire the moment the flame goes out of it - a puff this
        // small with a red heart reads as a spot, not as a fire dying.
        let ash = (0.3 + 0.55 * ((tau - hot) / (0.5 * life)).clamp(0.0, 1.0)).min(1.0);
        let shaded = if heat < pyro::FIREBALL { 0.0 } else { heat };
        puffs.push((heat, fire_puff(at, r, shaded, ash, true, 1.0 - thin * thin)));
        if hottest.is_none_or(|(h, _)| heat > h) {
            hottest = Some((heat, at));
        }
    }
    if let Some((heat, at)) = hottest.filter(|(h, _)| *h > 0.3) {
        glows.push(Shape::Glow { pos: at, radius: r0 * 2.6, color: pyro::alpha(FIRE[5], 0.2 * heat) });
    }
    puffs.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.extend(puffs.into_iter().map(|(_, p)| Shape::Puff(p)));

    // Fragments flung back toward the gun on short arcs.
    let m = 4 + (u(2) * 3.0) as u32;
    for e in 0..m {
        let s = 60 + e * 5;
        let span = 0.18 + 0.2 * u(s);
        if age < 0.01 || age >= span {
            continue;
        }
        let way = turn(back, (u(s + 1) - 0.5) * 2.2);
        let speed = (70.0 + 90.0 * u(s + 2)) * scale;
        let at = |t: f32| Position::new(pos.x + way.x * speed * t, pos.y + way.y * speed * t + 110.0 * t * t);
        let k = age / span;
        out.push(Shape::Line { from: at(age), to: at((age - 0.03).max(0.0)), width: 1.0, head: pyro::step(&FIRE[2..8], 1.0 - k), tail: pyro::alpha(FIRE[2], 0.4) });
    }

    // The flash: a white-hot core and a few short rays, over a light.
    let flash = 0.06;
    if age < flash {
        let k = 1.0 - age / flash;
        let core = r0 * (0.45 + 0.35 * (1.0 - k));
        out.push(Shape::Puff(Puff { pos, radius: core, body: FIRE[6], shadow: None, lit: None, core: Some((FIRE[7], 0.65)), cover: 1.0 }));
        let rays = 5;
        for ray in 0..rays {
            let way = turn(back, (ray as f32 / (rays - 1) as f32 - 0.5) * 2.6 + (u(80 + ray) - 0.5) * 0.3);
            let len = r0 * (1.4 + 0.8 * u(90 + ray)) * (0.6 + 0.4 * k);
            out.push(Shape::Line { from: pos, to: Position::new(pos.x + way.x * len, pos.y + way.y * len), width: 2.0, head: FIRE[7], tail: pyro::alpha(FIRE[5], 0.0) });
        }
    }
    if age < 0.2 {
        let k = 1.0 - age / 0.2;
        glows.push(Shape::Glow { pos, radius: r0 * (2.0 + 0.8 * (1.0 - k)), color: pyro::alpha(FIRE[6], 0.22 * k) });
    }
    out.extend(glows);
}

/// A spark star: a hot core and four rays - an upright plus for a hit, a
/// turned cross for a glance - gone in a few frames, with a wisp of smoke
/// after a hit.
fn star(out: &mut Vec<Shape>, pos: Position, age: f32, seed: u32, scale: f32, ramp: &[Color; 4], wisp: bool) {
    let flash = 0.07;
    if age < flash {
        let k = 1.0 - age / flash;
        let turned = if wisp { unit(seed, 1) < 0.5 } else { true };
        let reach = (4.0 + 5.0 * k) * scale;
        let step = if k > 0.5 { 0 } else { 1 };
        for q in 0..4 {
            let a = q as f32 * PI / 2.0 + if turned { PI / 4.0 } else { 0.0 };
            let tip = Position::new(pos.x + a.cos() * reach, pos.y + a.sin() * reach);
            out.push(Shape::Line { from: pos, to: tip, width: 1.0, head: ramp[step + 1], tail: pyro::alpha(ramp[3], 0.5) });
        }
        out.push(Shape::Mark { pos, size: if k > 0.5 { 4 } else { 2 }, color: ramp[step] });
        out.push(Shape::Glow { pos, radius: 12.0 * scale, color: pyro::alpha(ramp[2], 0.4 * k) });
    }
    if wisp && age > 0.03 {
        let q = ((age - 0.03) / 0.2).clamp(0.0, 1.0);
        if q < 1.0 {
            let at = Position::new(pos.x + (unit(seed, 2) - 0.5) * 4.0, pos.y - 6.0 * q * scale);
            out.push(Shape::Puff(Puff { pos: at, radius: (2.5 + 2.0 * q) * scale, body: SMOKE[3], shadow: None, lit: Some(SMOKE[4]), core: None, cover: 1.0 - q }));
        }
    }
}

/// A ring of energy racing out in `ramp`'s colour: a flash, the ring and
/// a fainter echo inside it, and forks of lightning crackling out to it -
/// or, `spiral`, arms turning inside it - over a light.
fn ring(out: &mut Vec<Shape>, pos: Position, t: f32, seed: u32, scale: f32, ramp: &[Color; 5], spiral: bool) {
    let reach = (5.0 + 26.0 * ease_out(t / 0.7)) * scale;
    let fade = 1.0 - t;
    let front = if t < 0.3 { ramp[3] } else if t < 0.65 { ramp[2] } else { ramp[1] };
    if t < 0.9 {
        pyro_ring(out, pos, reach, 0.62, 1, pyro::alpha(front, fade.max(0.35)));
        pyro_ring(out, pos, reach * 0.72, 0.62, 2, pyro::alpha(ramp[1], 0.7 * fade));
    }
    if spiral {
        // Three arms wound round the middle, turning as the ring spreads.
        for arm in 0..3u32 {
            let base = TAU * arm as f32 / 3.0 + t * 5.0 + unit(seed, 3) * TAU;
            let marks = (reach / BLOCK) as u32;
            for m in 0..marks {
                let k = m as f32 / marks.max(1) as f32;
                let a = base + k * 2.6;
                let r = reach * 0.8 * k;
                out.push(Shape::Mark { pos: Position::new(pos.x + a.cos() * r, pos.y + a.sin() * r * 0.62), size: 2, color: pyro::alpha(ramp[3], fade) });
            }
        }
    } else if t < 0.6 {
        // Forks of lightning from the middle out to the ring, re-jagged
        // every few frames so they crackle.
        let jolt = (t * 18.0) as u32;
        for fork in 0..4u32 {
            let s = 20 + fork * 13 + jolt * 131;
            let a = TAU * (fork as f32 + unit(seed, s)) / 4.0;
            let tip = Position::new(pos.x + a.cos() * reach, pos.y + a.sin() * reach * 0.62);
            jagged(out, pos, tip, seed.wrapping_add(s), 3.0 * scale, ramp[4], ramp[3]);
        }
    }
    if t < 0.12 {
        let k = 1.0 - t / 0.12;
        out.push(Shape::Puff(Puff { pos, radius: (5.0 + 3.0 * k) * scale, body: ramp[3], shadow: None, lit: None, core: Some((ramp[4], 0.6)), cover: 1.0 }));
    }
    out.push(Shape::Glow { pos, radius: 10.0 + reach * 0.8, color: pyro::alpha(ramp[2], 0.32 * fade) });
}

/// Marks round an ellipse `rx` wide and `rx * squash` tall, every
/// `stride`-th block.
fn pyro_ring(out: &mut Vec<Shape>, center: Position, rx: f32, squash: f32, stride: u32, color: Color) {
    if rx < BLOCK || color.a == 0 {
        return;
    }
    let steps = (TAU * rx / BLOCK).ceil().max(4.0) as u32;
    for i in (0..steps).step_by(stride.max(1) as usize) {
        let a = TAU * i as f32 / steps as f32;
        out.push(Shape::Mark { pos: Position::new(center.x + a.cos() * rx, center.y + a.sin() * rx * squash), size: 2, color });
    }
}

/// A jagged bolt from `a` to `b`: a kink every ~6 px pushed sideways by up
/// to `jag` px, the first half in `hot` and the rest in `warm`.
fn jagged(out: &mut Vec<Shape>, a: Position, b: Position, seed: u32, jag: f32, hot: Color, warm: Color) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let n = ((len / 6.0).round() as u32).max(2);
    let (nx, ny) = (-dy / len, dx / len);
    let mut prev = a;
    for i in 1..=n {
        let k = i as f32 / n as f32;
        let off = if i == n { 0.0 } else { (unit(seed, i) - 0.5) * 2.0 * jag };
        let next = Position::new(a.x + dx * k + nx * off, a.y + dy * k + ny * off);
        let c = if k <= 0.5 { hot } else { warm };
        out.push(Shape::Line { from: prev, to: next, width: 1.0, head: c, tail: c });
        prev = next;
    }
}

/// A laser's burn: a molten core in the beam's colour cooling to a dark
/// spot, droplets of slag splashing back toward the gun.
#[allow(clippy::too_many_arguments)]
fn laser(out: &mut Vec<Shape>, pos: Position, dir: Vec2, age: f32, t: f32, seed: u32, scale: f32, ramp: &[Color; 5]) {
    let back = Vec2::new(-dir.x, -dir.y);
    out.push(Shape::Puff(Puff::plain(pos, 3.0 * scale, pyro::alpha(CHAR[2], 0.8))));
    let r = (2.0 + 4.0 * (1.0 - t)) * scale;
    let body = if t < 0.4 { ramp[3] } else { ramp[2] };
    out.push(Shape::Puff(Puff { pos, radius: r, body, shadow: None, lit: None, core: Some((if t < 0.5 { ramp[4] } else { FIRE[6] }, 0.55)), cover: 1.0 }));
    for e in 0..6u32 {
        let s = 40 + e * 5;
        let span = 0.12 + 0.14 * unit(seed, s);
        if age >= span {
            continue;
        }
        let way = turn(back, (unit(seed, s + 1) - 0.5) * 2.4);
        let speed = (60.0 + 80.0 * unit(seed, s + 2)) * scale;
        let at = |t: f32| Position::new(pos.x + way.x * speed * t, pos.y + way.y * speed * t + 140.0 * t * t);
        let k = age / span;
        out.push(Shape::Line { from: at(age), to: at((age - 0.025).max(0.0)), width: 1.0, head: pyro::step(&FIRE[3..8], 1.0 - k), tail: pyro::alpha(FIRE[3], 0.5) });
    }
    out.push(Shape::Glow { pos, radius: 20.0 * scale, color: pyro::alpha(ramp[2], 0.55 * (1.0 - t)) });
}

/// A glob bursting on the ground: blobs of ooze flung out and settling, a
/// wet ring, a faint light.
fn ooze(out: &mut Vec<Shape>, pos: Position, age: f32, t: f32, seed: u32, scale: f32) {
    let n = 6;
    for i in 0..n {
        let s = 30 + i * 5;
        let a = TAU * (i as f32 + 0.5 * unit(seed, s)) / n as f32;
        let reach = (6.0 + 10.0 * unit(seed, s + 1)) * ease_out(age / 0.12) * scale;
        let r = (2.5 + 2.5 * unit(seed, s + 2)) * scale * (1.0 - 0.6 * t);
        if r < BLOCK {
            continue;
        }
        let at = Position::new(pos.x + a.cos() * reach, pos.y + a.sin() * reach * 0.7);
        out.push(Shape::Puff(Puff { pos: at, radius: r, body: OOZE[1], shadow: Some(OOZE[0]), lit: Some(OOZE[2]), core: None, cover: 1.0 - ((t - 0.6) / 0.4).clamp(0.0, 1.0) }));
    }
    if t < 0.6 {
        pyro_ring(out, pos, (4.0 + 18.0 * ease_out(t / 0.6)) * scale, 0.62, 2, pyro::alpha(OOZE[3], 1.0 - t / 0.6));
    }
    out.push(Shape::Glow { pos, radius: 16.0 * scale, color: pyro::alpha(OOZE[2], 0.25 * (1.0 - t)) });
}

/// A muzzle's flash, `age` seconds after the shot left, `dir` the way it
/// went (a unit vector; `None` for a launch with no single heading, which
/// flashes all round): a white-hot core at the muzzle, a tongue of flame
/// thrown down the line of the shot and short jets out to the sides,
/// stepping down the fire ramp and gone within `muzzle_flash_duration`.
pub fn muzzle(pos: Position, dir: Option<Vec2>, age: f32, seed: u32, plasma: Option<PlasmaVariant>) -> Vec<Shape> {
    let mut out = Vec::new();
    let life = tuning().muzzle_flash_duration.max(0.01);
    let t = age / life;
    if t >= 1.0 {
        return out;
    }
    let size = tuning().muzzle_glow_radius / 14.0;
    // Fire, white-hot to red; a plasma cannon's flash burns in its bolt's
    // own colour.
    let ramp: [Color; 5] = match plasma {
        Some(PlasmaVariant::Teal) => [PLASMA_TEAL[1], PLASMA_TEAL[2], PLASMA_TEAL[3], PLASMA_TEAL[4], PLASMA_TEAL[4]],
        Some(PlasmaVariant::Purple) => [PLASMA_PURPLE[1], PLASMA_PURPLE[2], PLASMA_PURPLE[3], PLASMA_PURPLE[4], PLASMA_PURPLE[4]],
        None => [FIRE[2], FIRE[3], FIRE[5], FIRE[6], FIRE[7]],
    };
    let step = (t * 3.0) as usize; // 0, 1, 2: white-hot, gold, red
    let (core, flame, edge) = [(ramp[4], ramp[3], ramp[2]), (ramp[3], ramp[2], ramp[1]), (ramp[2], ramp[1], ramp[0])][step.min(2)];
    let k = 1.0 - t;
    match dir {
        Some(d) => {
            let len = (8.0 + 8.0 * k + 3.0 * unit(seed, 1)) * size;
            let tip = Position::new(pos.x + d.x * len, pos.y + d.y * len);
            out.push(Shape::Line { from: pos, to: tip, width: 3.0 * k.max(0.34), head: flame, tail: edge });
            for side in [1.0, -1.0] {
                let way = turn(d, side * (1.1 + 0.3 * unit(seed, 2)));
                let short = (3.0 + 3.0 * k) * size;
                out.push(Shape::Line { from: pos, to: Position::new(pos.x + way.x * short, pos.y + way.y * short), width: 1.0, head: flame, tail: edge });
            }
        }
        None => {
            for q in 0..4 {
                let a = q as f32 * PI / 2.0 + PI / 4.0;
                let reach = (4.0 + 5.0 * k) * size;
                out.push(Shape::Line { from: pos, to: Position::new(pos.x + a.cos() * reach, pos.y + a.sin() * reach), width: 1.0, head: flame, tail: edge });
            }
        }
    }
    out.push(Shape::Puff(Puff { pos, radius: (3.0 + 2.0 * k) * size, body: flame, shadow: None, lit: None, core: Some((core, 0.6)), cover: 1.0 }));
    out.push(Shape::Glow { pos, radius: 18.0 * size, color: pyro::alpha(ramp[2], 0.42 * k) });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colours(shapes: &[Shape]) -> Vec<Color> {
        shapes
            .iter()
            .filter_map(|s| match *s {
                Shape::Puff(p) => Some(p.body),
                Shape::Mark { color, .. } => Some(color),
                Shape::Line { head, .. } => Some(head),
                Shape::Glow { .. } => None,
            })
            .collect()
    }

    fn opaque(c: Color) -> Color {
        Color::new(c.r, c.g, c.b, 255)
    }

    #[test]
    fn a_shell_hit_flashes_burns_and_is_smoke_by_the_end() {
        let at = Position::new(100.0, 100.0);
        let dir = Vec2::new(0.0, 1.0);
        let life = ImpactKind::Shell.seconds();
        let first = compose(ImpactKind::Shell, at, dir, 0.01, 7, 0.0);
        assert!(colours(&first).contains(&FIRE[6]), "a white-hot flash");
        let late = compose(ImpactKind::Shell, at, dir, life * 0.8, 7, 0.0);
        assert!(colours(&late).iter().all(|c| !FIRE[4..].contains(&opaque(*c))), "no flame left late on");
        assert!(colours(&late).iter().any(|c| SMOKE.contains(&opaque(*c))), "smoke hangs");
    }

    #[test]
    fn every_burst_is_drawn_in_its_ramp() {
        let at = Position::new(50.0, 50.0);
        let dir = Vec2::new(1.0, 0.0);
        let cases: [(ImpactKind, &[Color]); 5] = [
            (ImpactKind::Shell, &[&FIRE[..], &SMOKE[..], &CHAR[..]].concat()),
            (ImpactKind::Plasma(PlasmaVariant::Teal), &PLASMA_TEAL),
            (ImpactKind::Plasma(PlasmaVariant::Purple), &PLASMA_PURPLE),
            (ImpactKind::Tesla, &TESLA),
            (ImpactKind::Ooze, &OOZE),
        ];
        for (kind, ramp) in cases {
            for i in 0..10 {
                let age = kind.seconds() * i as f32 / 10.0;
                for c in colours(&compose(kind, at, dir, age, 3, 0.0)) {
                    assert!(ramp.contains(&opaque(c)), "{kind:?} at {age}: {c:?}");
                }
            }
        }
    }

    #[test]
    fn a_hit_throws_its_fire_back_toward_the_gun() {
        let at = Position::new(200.0, 200.0);
        let dir = Vec2::new(0.0, 1.0); // travelling down, so back is up
        let shapes = compose(ImpactKind::Shell, at, dir, 0.1, 11, 0.0);
        let ys: Vec<f32> = shapes.iter().filter_map(|s| if let Shape::Line { from, .. } = s { Some(from.y) } else { None }).collect();
        assert!(!ys.is_empty());
        let mean = ys.iter().sum::<f32>() / ys.len() as f32;
        assert!(mean < at.y, "fragments fly back up the shot's path: {mean}");
    }

    #[test]
    fn the_same_hit_always_looks_the_same() {
        let at = Position::new(64.0, 96.0);
        let dir = Vec2::new(0.0, -1.0);
        for kind in [ImpactKind::Shell, ImpactKind::Bullet, ImpactKind::Laser(false), ImpactKind::Plasma(PlasmaVariant::Teal)] {
            assert_eq!(compose(kind, at, dir, 0.05, 9, 0.2), compose(kind, at, dir, 0.05, 9, 0.2));
        }
    }

    #[test]
    fn a_tile_throws_its_own_dust_and_it_settles() {
        use crate::obstacle::Material;
        let at = Position::new(100.0, 100.0);
        let dir = Vec2::new(0.0, 1.0);
        for (material, kind) in [(Material::Sandbag, ImpactKind::Dust(Material::Sandbag)), (Material::Brick, ImpactKind::Collapse(Material::Brick))] {
            let ramp = pyro::dust_of(material).unwrap();
            let early = compose(kind, at, dir, kind.seconds() * 0.2, 5, 0.0);
            assert!(!early.is_empty(), "{kind:?} throws dust");
            for c in colours(&early) {
                assert!(ramp.contains(&opaque(c)), "{kind:?}: {c:?}");
            }
            assert!(compose(kind, at, dir, kind.seconds(), 5, 0.0).is_empty(), "{kind:?} has settled by the end");
        }
        assert!(compose(ImpactKind::Dust(Material::Iron), at, dir, 0.1, 5, 0.0).is_empty(), "iron sparks, it does not dust");
        let collapse = compose(ImpactKind::Collapse(Material::Wood), at, dir, 0.4, 5, 0.0).len();
        let chip = compose(ImpactKind::Dust(Material::Wood), at, dir, 0.1, 5, 0.0).len();
        assert!(collapse > chip, "a tile coming down throws more than a chip: {collapse} vs {chip}");
    }

    #[test]
    fn a_muzzle_flash_points_down_the_shot_and_is_gone_in_time() {
        let at = Position::new(100.0, 100.0);
        let shapes = muzzle(at, Some(Vec2::new(1.0, 0.0)), 0.0, 1, None);
        let reach = shapes.iter().filter_map(|s| if let Shape::Line { to, .. } = s { Some(to.x - at.x) } else { None }).fold(0.0, f32::max);
        assert!(reach > 8.0, "the tongue reaches down the line: {reach}");
        assert!(muzzle(at, None, tuning().muzzle_flash_duration, 1, None).is_empty());
    }
}
