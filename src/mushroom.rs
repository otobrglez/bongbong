//! The mushroom cloud a dying tank goes up in (docs/mushroom-cloud.md),
//! composed at draw time from pixel-block puffs rather than played from a
//! sheet: a flash, a fireball that climbs a stem and spreads into a cap
//! rolling over itself like a smoke ring, a skirt of dust racing out
//! along the ground, and embers arcing off. Every puff is a `pixel_disc`
//! with a darker rim, so it reads drawn on the same 2 px grid as the
//! sprites, and the time is continuous, so it animates every frame
//! instead of stepping through twelve.
//!
//! No two clouds match: `Cloud::new` hashes the height, cap size, lean,
//! roll speed, puff counts and pace from the blast's seed, and every puff
//! hashes its own place, size and cooling. Nothing draws RNG; `compose`
//! is a pure function of the cloud and its age, so it is tested headless
//! and `draw` only paints what it returns.

use crate::Position;
use crate::blast::hash_unit;
use crate::canvas::Canvas;
use crate::tuning::tuning;
use sola_raylib::prelude::Color;
use std::f32::consts::{PI, TAU};

/// Fire from white hot to embers - the palette's own steps
/// (docs/PALETTE.md: WHITE, GOLD_BRIGHT, RED_BRIGHT, RED_MD, RED_DEEP,
/// RED_DK).
const FIRE: [Color; 6] = [
    Color::new(0xFF, 0xFF, 0xFF, 255),
    Color::new(0xEE, 0xA3, 0x43, 255),
    Color::new(0xFF, 0x42, 0x1A, 255),
    Color::new(0xE4, 0x42, 0x19, 255),
    Color::new(0x9C, 0x35, 0x27, 255),
    Color::new(0x81, 0x2F, 0x27, 255),
];

/// Smoke from soot to pale ash (BLACK, STONE_DARKEST, STONE_SHADE,
/// STONE_DK, STONE_MD, STONE_LT).
const SMOKE: [Color; 6] = [
    Color::new(0x25, 0x25, 0x25, 255),
    Color::new(0x37, 0x37, 0x37, 255),
    Color::new(0x5A, 0x5A, 0x5A, 255),
    Color::new(0x7E, 0x7E, 0x7E, 255),
    Color::new(0x9E, 0x9E, 0x96, 255),
    Color::new(0xC1, 0xC1, 0xC1, 255),
];

/// Rim width (px) around every puff: one block.
const RIM: f32 = 2.0;

/// One cloud's hashed shape. Built once when the tank dies; the live
/// knobs are read then, so a tuning change affects the next kill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cloud {
    pub seed: u32,
    /// How long the whole cloud lives; the blast is done after this.
    pub seconds: f32,
    /// How high (px) the cap's centre climbs above the hull.
    height: f32,
    /// The cap's radius (px) once it has spread.
    cap: f32,
    /// Sideways drift per px of height, signed.
    wind: f32,
    /// How fast (rad/s) the cap rolls over itself.
    roll: f32,
    /// Vertical squash of everything that lies in the ground plane (the
    /// cap's ring, the dust skirt) - the tilt of the top-down view.
    squash: f32,
    stem_width: f32,
    /// Puff spacings per second the stem's puffs travel up it.
    flow: f32,
    /// Phase of the stem's wobble.
    wobble: f32,
    cap_puffs: u32,
    dome_puffs: u32,
    skirt_puffs: u32,
    embers: u32,
}

/// One disc to paint: `rim` first one block wider, then `fill`, then an
/// optional `highlight` up and left of centre for volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puff {
    pub pos: Position,
    pub radius: f32,
    pub fill: Color,
    pub rim: Option<Color>,
    pub highlight: Option<Color>,
}

fn ease_out(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x) * (1.0 - x) * (1.0 - x)
}

/// The colours of a puff: fire while `heat` > 0 (1 = white hot), then
/// smoke from soot toward ash as `ash` goes 0 -> 1. The rim is one step
/// darker, the highlight one step lighter.
fn shade(heat: f32, ash: f32) -> (Color, Color, Color) {
    if heat > 0.0 {
        let i = (((1.0 - heat) * FIRE.len() as f32) as usize).min(FIRE.len() - 1);
        let rim = if i + 1 < FIRE.len() { FIRE[i + 1] } else { SMOKE[0] };
        (FIRE[i], rim, FIRE[i.saturating_sub(1)])
    } else {
        let i = 1 + ((ash.clamp(0.0, 1.0) * (SMOKE.len() - 2) as f32) as usize).min(SMOKE.len() - 3);
        (SMOKE[i], SMOKE[i - 1], SMOKE[i + 1])
    }
}

/// How much of a puff is left once it starts to dissolve at `start`
/// (a fraction of the cloud's life): 1 until then, 0 a short while after.
fn dissolve(p: f32, start: f32) -> f32 {
    1.0 - ((p - start) / 0.14).clamp(0.0, 1.0)
}

impl Cloud {
    /// The cloud for a blast with this `seed`, sized by its `scale`.
    pub fn new(seed: u32, scale: f32) -> Self {
        let t = tuning();
        let u = |k: u32| hash_unit(seed, k);
        let cap = t.mushroom_cap_px * scale * (0.8 + 0.45 * u(3));
        Cloud {
            seed,
            seconds: t.mushroom_seconds * (0.85 + 0.3 * u(1)),
            height: t.mushroom_height_px * scale * (0.75 + 0.5 * u(2)),
            cap,
            wind: (u(4) - 0.5) * 0.5,
            roll: (1.4 + 1.8 * u(5)) * if u(14) < 0.5 { 1.0 } else { 0.8 },
            squash: 0.32 + 0.2 * u(10),
            stem_width: cap * (0.2 + 0.1 * u(11)),
            flow: 1.5 + 2.0 * u(12),
            wobble: u(13) * TAU,
            cap_puffs: 20 + (u(6) * 12.0) as u32,
            dome_puffs: 2 + (u(15) * 3.0) as u32,
            skirt_puffs: 14 + (u(8) * 10.0) as u32,
            embers: 8 + (u(9) * 12.0) as u32,
        }
    }

    pub fn done(&self, time: f32) -> bool {
        time >= self.seconds
    }

    fn u(&self, k: u32) -> f32 {
        hash_unit(self.seed, k)
    }

    /// Everything to paint `time` seconds after the kill at `base`, as
    /// layers in painting order (back to front). Each layer is rimmed as
    /// a whole before it is filled, so puffs in one layer merge into one
    /// outlined mass while the layers stay distinct.
    pub fn compose(&self, base: Position, time: f32) -> Vec<Vec<Puff>> {
        let d = self.seconds;
        let t = time.clamp(0.0, d);
        let p = t / d;
        let fire_life = 0.3 * d;

        // The head: climbs fast, then keeps drifting up; spreads as it goes.
        let h = self.height * (ease_out(t / (0.38 * d)) + 0.12 * p);
        let lean = self.wind * (0.6 + 0.8 * p);
        let head = Position::new(base.x + lean * h, base.y - h);
        let cap = self.cap * (0.3 + 0.7 * ease_out(t / (0.42 * d))) * (1.0 + 0.18 * p);
        let ring = cap * 0.72;
        let tube = cap * 0.32;

        let mut skirt_back = Vec::new();
        let mut skirt_front = Vec::new();
        let mut stem = Vec::new();
        let mut cap_back: Vec<(f32, Puff)> = Vec::new();
        let mut cap_front: Vec<(f32, Puff)> = Vec::new();
        let mut dome = Vec::new();
        let mut sparks = Vec::new();

        // The flash, a beat long, under everything that follows.
        let flash = 0.09;
        if t < flash {
            let k = 1.0 - t / flash;
            sparks.push(Puff { pos: base, radius: 6.0 + 14.0 * k, fill: FIRE[0], rim: Some(FIRE[1]), highlight: None });
            // Eight rays of single blocks, flung out as the flash fades.
            let spin = self.u(20) * TAU;
            for ray in 0..8 {
                let a = spin + ray as f32 * TAU / 8.0;
                let long = if ray % 2 == 0 { 1.0 } else { 0.6 };
                let mut dist = 10.0 + 14.0 * k;
                while dist < (18.0 + 26.0 * (1.0 - k)) * long + 12.0 * k {
                    let pos = Position::new(base.x + a.cos() * dist, base.y + a.sin() * dist);
                    let fill = if dist < 22.0 { FIRE[0] } else { FIRE[1] };
                    sparks.push(Puff { pos, radius: 1.0, fill, rim: None, highlight: None });
                    dist += 2.0;
                }
            }
        }

        // Dust thrown out along the ground, racing out and settling.
        if t > 0.03 {
            for j in 0..self.skirt_puffs {
                let s = 600 + j * 4;
                let a = TAU * j as f32 / self.skirt_puffs as f32 + self.u(s) * 0.5 + t * 0.15;
                let reach = self.cap * 1.15 * ease_out(t / (0.5 * d)) * (0.75 + 0.5 * self.u(s + 1));
                let r = self.cap * 0.26 * (0.7 + 0.6 * self.u(s + 2)) * (1.0 - 0.35 * p) * dissolve(p, 0.3 + 0.35 * self.u(s + 3));
                if r < 2.0 {
                    continue;
                }
                let pos = Position::new(base.x + a.cos() * reach, base.y + 6.0 + a.sin() * reach * self.squash);
                let (fill, rim, _) = shade(0.0, 0.6 + 0.35 * p);
                let puff = Puff { pos, radius: r, fill, rim: Some(rim), highlight: None };
                if a.sin() < 0.0 { skirt_back.push(puff) } else { skirt_front.push(puff) }
            }
        }

        // The stem: puffs ride up it on a conveyor, fed from the ground,
        // waisted in the middle and flared at the foot; it breaks up from
        // the bottom once the fire has gone out of it.
        let cutoff = ((p - 0.5) / 0.3).clamp(0.0, 1.0);
        let top = (h - tube * 0.4).max(1.0);
        let spacing = (self.stem_width * 0.8).max(3.0);
        let slots = (self.height * 1.2 / spacing).ceil() as u32 + 1;
        let period = slots as f32 * spacing;
        for k in 0..slots {
            let rise = (k as f32 * spacing + t * self.flow * spacing) % period;
            if rise > top {
                continue;
            }
            let frac = rise / top;
            if frac < cutoff {
                continue;
            }
            let wob = (frac * 9.0 + t * 4.0 + self.wobble).sin() * 2.0;
            let pos = Position::new(base.x + lean * rise + wob, base.y - rise);
            let feed = (frac / 0.12).clamp(0.0, 1.0);
            let flare = 1.0 + 0.6 * (1.0 - frac).powi(3);
            let waist = 1.0 - 0.3 * (frac * PI).sin();
            let edge = ((frac - cutoff) / 0.15).clamp(0.0, 1.0);
            let r = self.stem_width * flare * waist * feed * (1.0 - 0.4 * p) * if cutoff > 0.0 { edge } else { 1.0 };
            if r < 2.0 {
                continue;
            }
            let hot = fire_life * (1.1 + 0.8 * (1.0 - frac));
            let heat = 1.0 - t / hot;
            let ash = 0.2 + ((t - hot) / (0.6 * d)).clamp(0.0, 1.0) * 0.5;
            let (fill, rim, _) = shade(heat, ash);
            stem.push(Puff { pos, radius: r, fill, rim: Some(rim), highlight: None });
        }

        // The cap: a ring of puffs rolling over itself like a smoke ring -
        // out over the top, down the outside, back in underneath - seen
        // from above at the view's tilt. The underside keeps its fire
        // longest; the top cools first and pales toward ash.
        for i in 0..self.cap_puffs {
            let s = 100 + i * 6;
            let az = TAU * i as f32 / self.cap_puffs as f32 + self.u(s) * 0.45;
            let roll = self.u(s + 1) * TAU - self.roll * t;
            let radial = ring + tube * roll.cos();
            let up = tube * roll.sin() * 0.75;
            let depth = az.sin();
            let pos = Position::new(head.x + radial * az.cos(), head.y - up + radial * depth * self.squash);
            let r = tube * (0.85 + 0.45 * self.u(s + 2)) * dissolve(p, 0.56 + 0.28 * self.u(s + 3));
            if r < 2.0 {
                continue;
            }
            let under = roll.sin() < 0.0;
            let hot = fire_life * (0.55 + 0.8 * self.u(s + 4)) * if under { 1.5 } else { 1.0 };
            let heat = 1.0 - t / hot;
            let ash = 0.2 + 0.8 * ((t - hot) / (0.55 * d)).clamp(0.0, 1.0) * (0.55 + 0.45 * (roll.sin() * 0.5 + 0.5));
            let (fill, rim, hi) = shade(heat, ash);
            let front = depth >= 0.0;
            let puff = Puff { pos, radius: r, fill, rim: Some(rim), highlight: (front && !under).then_some(hi) };
            if front { cap_front.push((depth, puff)) } else { cap_back.push((depth, puff)) }
        }
        // Back to front, so the near side of the ring covers the far side.
        let by_depth = |a: &(f32, Puff), b: &(f32, Puff)| a.0.total_cmp(&b.0);
        cap_back.sort_by(by_depth);
        cap_front.sort_by(by_depth);

        // The dome the ring rolls around, bulging up out of its middle.
        for i in 0..self.dome_puffs {
            let s = 300 + i * 5;
            let pos = Position::new(
                head.x + (self.u(s) - 0.5) * cap * 0.9,
                head.y - tube * (0.35 + 0.45 * self.u(s + 1)) + (t * 3.0 + self.u(s + 2) * TAU).sin() * 1.5,
            );
            let r = tube * (0.9 + 0.35 * self.u(s + 3)) * dissolve(p, 0.6 + 0.24 * self.u(s + 4));
            if r < 2.0 {
                continue;
            }
            let hot = fire_life * (0.45 + 0.4 * self.u(s + 4));
            let heat = 1.0 - t / hot;
            let ash = 0.3 + 0.7 * ((t - hot) / (0.5 * d)).clamp(0.0, 1.0);
            let (fill, rim, hi) = shade(heat, ash);
            dome.push(Puff { pos, radius: r, fill, rim: Some(rim), highlight: Some(hi) });
        }

        // Embers thrown up and out, falling back under the view's gravity.
        for e in 0..self.embers {
            let s = 800 + e * 5;
            let life = 0.45 + 0.9 * self.u(s);
            if t < flash || t >= life {
                continue;
            }
            let a = -PI / 2.0 + (self.u(s + 1) - 0.5) * 2.4;
            let speed = (60.0 + 120.0 * self.u(s + 2)) * self.cap / 30.0;
            let pos = Position::new(base.x + a.cos() * speed * t, base.y + a.sin() * speed * t + 110.0 * t * t);
            let (fill, _, _) = shade(1.0 - 0.2 - 0.8 * t / life, 0.0);
            let radius = if self.u(s + 3) < 0.3 { 2.0 } else { 1.0 };
            sparks.push(Puff { pos, radius, fill, rim: None, highlight: None });
        }

        vec![
            skirt_back,
            stem,
            cap_back.into_iter().map(|(_, p)| p).collect(),
            dome,
            cap_front.into_iter().map(|(_, p)| p).collect(),
            skirt_front,
            sparks,
        ]
    }
}

/// Paint the cloud of a kill at `base`, `time` seconds in. Through a
/// `Canvas`, so the headless tests and previews paint what the game does.
pub fn draw(c: &mut impl Canvas, cloud: &Cloud, base: Position, time: f32) {
    for layer in cloud.compose(base, time) {
        for puff in &layer {
            if let Some(rim) = puff.rim {
                block_disc(c, puff.pos, puff.radius + RIM, rim);
            }
        }
        for puff in &layer {
            if puff.rim.is_none() && puff.radius <= 2.0 {
                // An ember: one block, or a 2 x 2 of them.
                let side = (puff.radius * 2.0) as i32;
                let x = (puff.pos.x / BLOCK).floor() * BLOCK;
                let y = (puff.pos.y / BLOCK).floor() * BLOCK;
                c.fill_rect(x as i32, y as i32, side, side, puff.fill);
            } else {
                block_disc(c, puff.pos, puff.radius, puff.fill);
            }
        }
        for puff in &layer {
            if let Some(hi) = puff.highlight {
                let at = Position::new(puff.pos.x - puff.radius * 0.3, puff.pos.y - puff.radius * 0.35);
                block_disc(c, at, puff.radius * 0.45, hi);
            }
        }
    }
}

/// The block every puff is built from: the 2 screen px one sprite pixel
/// covers.
const BLOCK: f32 = 2.0;

/// `blast::pixel_disc` on a `Canvas`: a filled disc of whole blocks, one
/// scanline of blocks at a time, centre snapped to the grid.
fn block_disc(c: &mut impl Canvas, center: Position, radius: f32, color: Color) {
    if radius < BLOCK {
        return;
    }
    let cx = (center.x / BLOCK).round() * BLOCK;
    let cy = (center.y / BLOCK).round() * BLOCK;
    let rows = (radius / BLOCK).floor() as i32;
    for row in -rows..=rows {
        let dy = row as f32 * BLOCK;
        let half = (radius * radius - dy * dy).max(0.0).sqrt();
        let cols = (half / BLOCK).floor() as i32;
        if cols <= 0 {
            continue;
        }
        c.fill_rect(
            (cx - cols as f32 * BLOCK - BLOCK / 2.0) as i32,
            (cy + dy - BLOCK / 2.0) as i32,
            (cols * 2 + 1) * BLOCK as i32,
            BLOCK as i32,
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud(seed: u32) -> Cloud {
        Cloud::new(seed, 1.0)
    }

    fn top(c: &Cloud, time: f32) -> f32 {
        c.compose(Position::new(200.0, 200.0), time)
            .iter()
            .flatten()
            .filter(|p| p.rim.is_some())
            .map(|p| p.pos.y - p.radius)
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn the_cloud_climbs_spreads_and_is_gone_at_the_end() {
        let c = cloud(0xB0B5);
        let base = Position::new(200.0, 200.0);
        assert!(!c.compose(base, 0.0).iter().flatten().next().is_none(), "the flash is up on the first frame");
        assert!(top(&c, c.seconds * 0.5) < top(&c, 0.05) - 40.0, "the cap climbs well above the hull");
        let fire = |time: f32| c.compose(base, time).iter().flatten().filter(|p| FIRE.contains(&p.fill)).count();
        assert!(fire(0.2) > fire(c.seconds * 0.8), "the fire burns out into smoke");
        let late = c.compose(base, c.seconds * 0.999);
        assert!(late.iter().flatten().all(|p| p.radius < 2.0 + 1e-3 || p.rim.is_none()), "every puff has dissolved: {late:?}");
    }

    #[test]
    fn no_two_clouds_look_alike_and_one_cloud_always_looks_the_same() {
        let base = Position::new(300.0, 150.0);
        let a = cloud(1).compose(base, 0.8);
        assert_eq!(a, cloud(1).compose(base, 0.8));
        let shapes: std::collections::HashSet<(u32, u32, u32)> = (0..40)
            .map(|s| {
                let c = cloud(crate::blast::seed_for(Position::new(s as f32 * 37.0, 90.0)));
                (c.cap_puffs, c.skirt_puffs, (c.height / 4.0) as u32)
            })
            .collect();
        assert!(shapes.len() >= 20, "forty kills give many shapes: {shapes:?}");
    }

    #[test]
    fn the_cap_rolls_so_consecutive_frames_differ() {
        let c = cloud(7);
        let base = Position::new(100.0, 100.0);
        assert_ne!(c.compose(base, 1.0), c.compose(base, 1.0 + 1.0 / 60.0));
    }

    /// `cargo test --lib mushroom::tests::preview -- --ignored` writes
    /// target/mushroom_preview.png: six clouds (rows) over their life
    /// (columns), on a ground-green field, doubled.
    #[test]
    #[ignore]
    fn preview() {
        let (cols, rows, cell_w, cell_h) = (14, 6, 150, 190);
        let mut c = crate::canvas::CpuCanvas::blank(cols * cell_w, rows * cell_h);
        c.clear(Color::new(0x5e, 0x80, 0x3c, 255));
        for r in 0..rows {
            let cl = cloud(crate::blast::seed_for(Position::new(r as f32 * 64.0 + 32.0, 96.0)));
            for k in 0..cols {
                let t = cl.seconds * k as f32 / (cols - 1) as f32 * 0.97;
                let base = Position::new((k * cell_w + cell_w / 2) as f32, (r * cell_h + cell_h - 40) as f32);
                c.fill_rect(base.x as i32 - 14, base.y as i32 - 10, 28, 20, Color::new(0x44, 0x44, 0x44, 255));
                draw(&mut c, &cl, base, t);
            }
        }
        c.write_png(std::path::Path::new("target/mushroom_preview.png"), 2).unwrap();
        // And the same clouds in motion, one PNG per 1/30 s, for a GIF.
        let dir = std::path::Path::new("target/mushroom_frames");
        std::fs::create_dir_all(dir).unwrap();
        let clouds: Vec<Cloud> = (0..4).map(|r| cloud(crate::blast::seed_for(Position::new(r as f32 * 64.0 + 32.0, 96.0)))).collect();
        let longest = clouds.iter().map(|c| c.seconds).fold(0.0, f32::max);
        for f in 0..((longest + 0.3) * 30.0) as usize {
            let t = f as f32 / 30.0;
            let mut c = crate::canvas::CpuCanvas::blank(4 * 140, 200);
            c.clear(Color::new(0x5e, 0x80, 0x3c, 255));
            for (i, cl) in clouds.iter().enumerate() {
                let base = Position::new(70.0 + i as f32 * 140.0, 160.0);
                c.fill_rect(base.x as i32 - 14, base.y as i32 - 10, 28, 20, Color::new(0x44, 0x44, 0x44, 255));
                if !cl.done(t) {
                    draw(&mut c, cl, base, t);
                }
            }
            c.write_png(&dir.join(format!("{f:03}.png")), 2).unwrap();
        }
    }
}
