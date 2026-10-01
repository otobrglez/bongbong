//! The fireball every explosion but a mushroom cloud goes up in - a drum,
//! a plain tank kill, a missile's burst, a cook-off (docs/effects.md) -
//! composed at draw time in the effects language (`pyro.rs`) rather than
//! played from a sheet: a white flash with a few short rays, a cluster of
//! shaded puffs that burn from white through gold and red and cool into
//! smoke, the smoke climbing and leaning down-wind with the grass, hot
//! fragments on ballistic arcs, and dust racing out across the ground.
//! Everything is whole 2 px blocks in the fire, smoke and dust ramps, and
//! its light is stepped glows.
//!
//! The shape follows the cause `BlastFx::shaped` hashed into the blast's
//! row: a round ball, a column (`BLAST_ROW_TALL`: a fuel drum, a ram), a
//! flat burst thrown downrange (`BLAST_ROW_FLAT`, leaning along the shot),
//! or a double core (`BLAST_ROW_DOUBLE`). A fuel drum burns hotter and
//! cleaner than oil, whose smoke is black. Every puff hashes its place,
//! size and cooling from the blast's seed; nothing draws RNG, and
//! `compose` is a pure function of the blast, its age and the wind, so it
//! is tested headless and the painters only paint what it returns.

use crate::blast::{BlastFx, BlastKind};
use crate::pyro::{self, dust_puff, ease_out, fire_puff, unit, Puff, Shape, BLOCK, FIRE, SMOKE};
use crate::tuning::tuning;
use crate::{Position, BLAST_ROW_DOUBLE, BLAST_ROW_FLAT, BLAST_ROW_TALL};
use std::f32::consts::{PI, TAU};

/// The outline a cause gives the fire.
#[derive(Clone, Copy, Debug)]
struct Form {
    /// Spread of the puffs across and up the field (1 = round).
    sx: f32,
    sy: f32,
    /// How fast the fire climbs, as a multiple of the round ball's.
    rise: f32,
    /// How much smoke it leaves, as a multiple of the round ball's.
    smoke: f32,
    /// A second core beside the first, a moment later.
    twin: bool,
}

fn form(row: i32) -> Form {
    match row {
        BLAST_ROW_TALL => Form { sx: 0.72, sy: 1.35, rise: 1.9, smoke: 0.8, twin: false },
        BLAST_ROW_FLAT => Form { sx: 1.45, sy: 0.62, rise: 0.45, smoke: 0.6, twin: false },
        BLAST_ROW_DOUBLE => Form { sx: 0.9, sy: 0.85, rise: 1.0, smoke: 1.0, twin: true },
        _ => Form { sx: 1.0, sy: 1.0, rise: 1.0, smoke: 1.0, twin: false },
    }
}

/// The fireball's radius (px) at its peak: `blast_fireball_px` at the
/// blast's scale.
fn radius(fx: &BlastFx) -> f32 {
    tuning().blast_fireball_px * fx.scale
}

/// How long this blast's fire and smoke play: `blast_fireball_seconds`,
/// paced by its hashed rate, quicker for a fuel drum, and shorter the
/// smaller it is - a cook-off is a pop, not a second kill.
pub fn seconds(fx: &BlastFx) -> f32 {
    let base = tuning().blast_fireball_seconds.max(0.1);
    let pace = 1.0 / fx.fps_scale.max(0.1);
    let fuel = if fx.kind == BlastKind::Fuel { 0.88 } else { 1.0 };
    base * pace * fuel * (0.55 + 0.45 * fx.scale.min(1.4))
}

/// How long the flash lasts, seconds: a few frames, whatever the size.
const FLASH: f32 = 0.09;

/// How long the dust takes to race out and settle, seconds.
const DUST_OUT: f32 = 0.42;

/// Everything to paint `fx.time` seconds after it went off, back to
/// front. `lean` is how far the smoke drifts sideways per px it rises
/// (`pyro::smoke_lean` at the blast).
pub fn compose(fx: &BlastFx, lean: f32) -> Vec<Shape> {
    let life = seconds(fx);
    let t = fx.time.clamp(0.0, life);
    let r_peak = radius(fx);
    let shape = form(fx.row);
    let fuel = fx.kind == BlastKind::Fuel;
    let u = |k: u32| unit(fx.seed, k);
    let center = fx.center;
    // The hashed layout, turned by the blast's quarter-turns.
    let spin = u(2) * TAU + fx.turn as f32 * PI / 2.0;

    // Downrange: a shot throws the fire along its path, stretching the
    // cluster that way (`BlastFx::offset`, 0 for a blast with no cause).
    let (ox, oy) = (fx.offset.x, fx.offset.y);
    let olen = (ox * ox + oy * oy).sqrt();
    let along = if olen > 0.5 { Some((ox / olen, oy / olen)) } else { None };
    let stretch = |dx: f32, dy: f32| -> (f32, f32) {
        let (dx, dy) = (dx * shape.sx, dy * shape.sy);
        match along {
            Some((ax, ay)) => {
                let a = dx * ax + dy * ay;
                let c = -dx * ay + dy * ax;
                let (a, c) = (a * 1.35, c * 0.8);
                (a * ax - c * ay, a * ay + c * ax)
            }
            None => (dx, dy),
        }
    };

    let mut back = Vec::new();
    let mut fire: Vec<(f32, f32, Puff)> = Vec::new();
    let mut front = Vec::new();
    let mut glows = Vec::new();

    // Dust thrown out along the ground: a ring of low, thin puffs racing
    // out under the fireball, the far half behind it and the near half in
    // front, settling as it goes. A cook-off is too small to lift any.
    if !fx.secondary && t < DUST_OUT * 1.5 {
        let n = 7 + (u(40) * 4.0) as u32;
        let out = ease_out(t / DUST_OUT);
        for j in 0..n {
            let s = 400 + j * 5;
            let a = TAU * (j as f32 + 0.4 * u(s)) / n as f32 + spin;
            let reach = r_peak * (0.6 + 1.5 * out) * (0.8 + 0.4 * u(s + 1));
            let settle = 1.0 - ((t - DUST_OUT * (0.5 + 0.5 * u(s + 2))) / (DUST_OUT * 0.7)).clamp(0.0, 1.0);
            let r = r_peak * (0.2 + 0.1 * u(s + 3)) * (0.6 + 0.4 * out) * settle;
            if r < BLOCK {
                continue;
            }
            let pos = Position::new(center.x + a.cos() * reach, center.y + 4.0 + a.sin() * reach * 0.5);
            let puff = Shape::Puff(dust_puff(pos, r, u(s + 4), 1.0));
            if a.sin() < 0.0 { back.push(puff) } else { front.push(puff) }
        }
    }

    // The fireball: puffs flung out from the core, each burning out on
    // its own clock (the middle ones last) and cooling into smoke that
    // climbs and leans with the wind, then shrinks away.
    let n = (((8.0 + 5.0 * u(1)) * (0.55 + 0.45 * fx.scale.min(1.3))).round() as u32).max(4);
    // Heat-weighted sums of the burning puffs (weight, x, y) and the
    // hottest heat, for the fire's one light.
    let mut lit = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for i in 0..n {
        let s = 100 + i * 7;
        let twin = shape.twin && i % 2 == 1;
        let born = 0.05 * u(s) + if twin { 0.05 } else { 0.0 };
        let tau = t - born;
        if tau <= 0.0 {
            continue;
        }
        let a = TAU * (i as f32 + 0.45 * u(s + 1)) / n as f32 + spin;
        let d = r_peak * (0.1 + 0.55 * u(s + 2));
        let (dx, dy) = stretch(a.cos() * d, a.sin() * d);
        let mut base = center;
        if twin {
            let ta = u(3) * TAU;
            let (tx, ty) = stretch(ta.cos() * r_peak * 0.5, ta.sin() * r_peak * 0.5);
            base = Position::new(base.x + tx, base.y + ty);
        }
        let grow = ease_out(tau / 0.16);
        let inner = 1.0 - d / (r_peak * 0.65);
        let hot = life * (0.16 + 0.2 * u(s + 3)) * (0.8 + 0.6 * inner) * if fuel { 1.25 } else { 1.0 };
        let heat = (1.0 - tau / hot).clamp(0.0, 1.0).powf(0.85);
        // Climbs slowly while it burns, then faster as smoke.
        let q = tau / life;
        let climb = r_peak * shape.rise * (0.25 * q + 1.1 * q * q) * (0.7 + 0.6 * u(s + 4));
        let pos = Position::new(
            base.x + ox * (0.4 + 0.6 * grow) + dx * (0.35 + 0.65 * grow) + lean * climb,
            base.y + oy * (0.4 + 0.6 * grow) + dy * (0.35 + 0.65 * grow) - climb,
        );
        // The smoke thins away through the Bayer pattern over the end of
        // its life, shrinking a little as it goes.
        let gone = 0.58 + 0.2 * u(s + 5);
        let thin = ((q - gone) / (1.0 - gone).max(0.05)).clamp(0.0, 1.0);
        let swell = 1.0 + 0.45 * q * shape.smoke;
        let r = r_peak * (0.3 + 0.2 * u(s + 6)) * (0.45 + 0.55 * grow) * swell * (1.0 - 0.6 * thin);
        if r < BLOCK || thin >= 1.0 {
            continue;
        }
        // Oil burns black, fuel leaves a paler, thinner smoke.
        let (soot, pale) = if fuel { (0.3, 0.85) } else { (0.0, 0.6) };
        let ash = (soot + 0.15 + pale * ((tau - hot) / (0.6 * life)).clamp(0.0, 1.0)).min(1.0);
        // The smoke breaks up as it goes: shrinking, thinning only at the
        // very end.
        let puff = fire_puff(pos, r, heat, ash, true, 1.0 - thin * thin);
        if heat > 0.35 {
            lit.0 += heat;
            lit.1 += pos.x * heat;
            lit.2 += pos.y * heat;
            lit.3 = lit.3.max(heat);
        }
        fire.push((heat, pos.y, puff));
    }
    // The fire's light: one glow over where it burns hottest, following
    // it up a column - not one per puff, which pile up into a white blot.
    let (w, wx, wy, hottest) = lit;
    if w > 0.0 {
        glows.push(Shape::Glow { pos: Position::new(wx / w, wy / w), radius: r_peak * 2.0, color: pyro::alpha(FIRE[5], 0.32 * hottest) });
    }
    // Cold behind hot: the smoke under what still burns, the hottest last.
    fire.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));

    // Fragments on ballistic arcs: hot ones cooling down the fire ramp,
    // a few dark chunks, each a short streak behind where it is.
    let mut debris = Vec::new();
    if fx.scale >= 0.5 {
        let m = 6 + (u(50) * 7.0) as u32;
        for e in 0..m {
            let s = 800 + e * 5;
            let span = 0.35 + 0.5 * u(s);
            if t < 0.02 || t >= span {
                continue;
            }
            let a = -PI / 2.0 + (u(s + 1) - 0.5) * 3.2;
            let speed = (80.0 + 140.0 * u(s + 2)) * fx.scale;
            let at = |t: f32| Position::new(center.x + ox + a.cos() * speed * t, center.y + oy + a.sin() * speed * t + 150.0 * t * t);
            let k = t / span;
            let dark = u(s + 3) < 0.35;
            let (head, tail) = if dark {
                (SMOKE[0], pyro::alpha(SMOKE[1], 0.5))
            } else {
                (pyro::step(&FIRE[2..8], 1.0 - k), pyro::alpha(FIRE[2], 0.4))
            };
            debris.push(Shape::Line { from: at(t), to: at((t - 0.035).max(0.0)), width: 1.0, head, tail });
        }
    }

    // The flash: a white core swelling for a frame and a few short rays,
    // gone in a few frames, over a wide light.
    let mut flash = Vec::new();
    if t < FLASH {
        let k = 1.0 - t / FLASH;
        let core = r_peak * (0.28 + 0.3 * (1.0 - k)) * k.sqrt();
        let hub = Position::new(center.x + ox * 0.5, center.y + oy * 0.5);
        flash.push(Shape::Puff(Puff { pos: hub, radius: core * 1.4, body: FIRE[6], shadow: None, lit: None, core: Some((FIRE[7], 0.7)), cover: 1.0 }));
        if t < FLASH * 0.75 {
            let rays = 6 + (u(60) * 3.0) as u32;
            for ray in 0..rays {
                let a = TAU * (ray as f32 + 0.5 * u(61 + ray)) / rays as f32 + spin;
                let len = r_peak * (0.9 + 0.8 * u(70 + ray)) * (0.6 + 0.4 * k);
                let tip = Position::new(hub.x + a.cos() * len, hub.y + a.sin() * len * 0.8);
                flash.push(Shape::Line { from: hub, to: tip, width: 2.0, head: FIRE[7], tail: pyro::alpha(FIRE[5], 0.0) });
            }
        }
    }
    // The bloom the blast opens with, swelling as it fades.
    let tn = tuning();
    let bloom = tn.blast_glow_seconds;
    if bloom > 0.0 && t < bloom {
        let k = 1.0 - t / bloom;
        let strength = tn.blast_glow_strength * if fuel { 1.25 } else { 1.0 };
        let r = tn.blast_glow_radius * fx.scale * (0.9 + 0.6 * (1.0 - k));
        glows.push(Shape::Glow { pos: center, radius: r, color: pyro::alpha(FIRE[6], strength * k) });
    }

    let mut out = back;
    out.extend(fire.into_iter().map(|(_, _, puff)| Shape::Puff(puff)));
    out.extend(front);
    out.extend(debris);
    out.extend(flash);
    out.extend(glows);
    out
}

/// Is the blast over? When its last smoke has gone.
pub fn done(fx: &BlastFx) -> bool {
    fx.time >= seconds(fx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blast::BlastShape;
    use crate::pyro::DUST;

    fn puffs(shapes: &[Shape]) -> Vec<Puff> {
        shapes.iter().filter_map(|s| if let Shape::Puff(p) = s { Some(*p) } else { None }).collect()
    }

    fn at(mut fx: BlastFx, t: f32) -> Vec<Shape> {
        fx.time = t;
        compose(&fx, 0.0)
    }

    #[test]
    fn it_flashes_burns_cools_into_smoke_and_is_gone() {
        let fx = BlastFx::new(Position::new(300.0, 200.0));
        let first = at(BlastFx::new(fx.center), 0.01);
        assert!(puffs(&first).iter().any(|p| p.body == FIRE[6] || p.core.is_some_and(|(c, _)| c == FIRE[7])), "a white-hot flash");
        let burning = at(BlastFx::new(fx.center), 0.15);
        assert!(puffs(&burning).iter().any(|p| FIRE.contains(&p.body)), "fire at 0.15 s");
        let late = seconds(&fx) * 0.8;
        let smoky = at(BlastFx::new(fx.center), late);
        assert!(!puffs(&smoky).is_empty(), "smoke still hangs at 80%");
        assert!(puffs(&smoky).iter().all(|p| !FIRE[3..].contains(&p.body)), "no flame left at 80%");
        let mut over = BlastFx::new(fx.center);
        over.time = seconds(&over);
        assert!(done(&over));
    }

    #[test]
    fn every_colour_is_a_ramp_step_and_every_block_is_on_the_grid() {
        for kind in [BlastKind::Oil, BlastKind::Fuel] {
            for shape in [BlastShape::Plain, BlastShape::Ram, BlastShape::Shot { dir: crate::blast::Lean { x: 0.0, y: -1.0 } }] {
                let fx = BlastFx::shaped(Position::new(211.0, 123.0), kind, shape);
                let life = seconds(&fx);
                for i in 0..24 {
                    for s in at(BlastFx::shaped(fx.center, kind, shape), life * i as f32 / 24.0) {
                        if let Shape::Puff(p) = s {
                            assert!(FIRE.contains(&p.body) || SMOKE.contains(&p.body) || DUST.contains(&p.body), "{:?}", p.body);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_column_climbs_higher_than_a_flat_burst() {
        let top = |fx: BlastFx| {
            let t = seconds(&fx) * 0.6;
            puffs(&at(fx, t)).iter().map(|p| p.pos.y - p.radius).fold(f32::MAX, f32::min)
        };
        let c = Position::new(400.0, 300.0);
        let tall = top(BlastFx::shaped(c, BlastKind::Fuel, BlastShape::Plain));
        let flat = top(BlastFx::shaped(c, BlastKind::Oil, BlastShape::Shot { dir: crate::blast::Lean { x: 1.0, y: 0.0 } }));
        assert!(tall < flat - 10.0, "column top {tall}, flat top {flat}");
    }

    #[test]
    fn the_smoke_leans_down_wind() {
        let fx = BlastFx::new(Position::new(400.0, 300.0));
        let t = seconds(&fx) * 0.7;
        let mean_x = |lean: f32| {
            let mut f = BlastFx::new(fx.center);
            f.time = t;
            let ps = puffs(&compose(&f, lean));
            ps.iter().map(|p| p.pos.x).sum::<f32>() / ps.len().max(1) as f32
        };
        assert!(mean_x(0.4) > mean_x(0.0) + 3.0, "an east wind carries the smoke east");
        assert!(mean_x(-0.4) < mean_x(0.0) - 3.0);
    }

    #[test]
    fn a_cookoff_is_a_small_quick_pop() {
        let c = Position::new(100.0, 100.0);
        let small = BlastFx::small(c);
        assert!(seconds(&small) < seconds(&BlastFx::new(c)) * 0.8);
        let biggest = |fx: BlastFx| puffs(&at(fx, 0.12)).iter().map(|p| p.radius).fold(0.0, f32::max);
        assert!(biggest(BlastFx::small(c)) < biggest(BlastFx::new(c)) * 0.7);
    }

    #[test]
    fn the_same_blast_always_looks_the_same_and_two_blasts_differ() {
        let a = at(BlastFx::new(Position::new(320.0, 160.0)), 0.2);
        let b = at(BlastFx::new(Position::new(320.0, 160.0)), 0.2);
        let c = at(BlastFx::new(Position::new(352.0, 160.0)), 0.2);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    /// `cargo test --lib fireball::tests::preview -- --ignored` writes
    /// target/fireball_preview.png: one blast of each kind (oil, fuel,
    /// shot, ram, double, missile, cook-off) across its life, doubled.
    #[test]
    #[ignore]
    #[cfg(feature = "render")] // write_png is raylib's PNG encoder
    fn preview() {
        use crate::canvas::{Canvas, CpuCanvas};
        use crate::math::Color;
        let ground = Color::new(0x61, 0x95, 0x41, 255);
        let kinds: Vec<(&str, Box<dyn Fn(Position) -> BlastFx>)> = vec![
            ("oil", Box::new(|c| BlastFx::shaped(c, BlastKind::Oil, BlastShape::Plain))),
            ("fuel", Box::new(|c| BlastFx::shaped(c, BlastKind::Fuel, BlastShape::Plain))),
            ("shot", Box::new(|c| BlastFx::shaped(c, BlastKind::Oil, BlastShape::Shot { dir: crate::blast::Lean { x: 1.0, y: 0.0 } }))),
            ("ram", Box::new(|c| BlastFx::shaped(c, BlastKind::Oil, BlastShape::Ram))),
            ("missile", Box::new(|c| {
                let mut fx = BlastFx::shaped(c, BlastKind::Oil, BlastShape::Shot { dir: crate::blast::Lean { x: 0.0, y: -1.0 } });
                fx.scale *= 0.55;
                fx
            })),
            ("cookoff", Box::new(BlastFx::small)),
        ];
        let (cols, cell_w, cell_h) = (12, 150, 170);
        let mut c = CpuCanvas::blank((cols * cell_w) as usize, kinds.len() * cell_h as usize);
        c.clear(ground);
        for (r, (_, make)) in kinds.iter().enumerate() {
            for k in 0..cols {
                let base = Position::new((k * cell_w + cell_w / 2) as f32, (r as i32 * cell_h + cell_h - 50) as f32);
                let mut fx = make(base);
                let life = seconds(&fx);
                fx.time = if k == 0 { 0.02 } else { life * (k as f32 / cols as f32) };
                c.fill_rect(base.x as i32 - 14, base.y as i32 - 10, 28, 20, Color::new(0x44, 0x44, 0x44, 255));
                let shapes = compose(&fx, 0.25);
                pyro::draw(&mut c, &shapes);
                pyro::draw_glows(&mut c, &shapes, 4);
            }
        }
        c.write_png(std::path::Path::new("target/fireball_preview.png"), 1).unwrap();
    }
}
