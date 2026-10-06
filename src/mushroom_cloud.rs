//! The mushroom cloud a dying tank goes up in (docs/mushroom-cloud.md),
//! composed at draw time rather than played from a sheet, in the effects
//! language every explosion shares (`pyro.rs`, docs/effects.md). The look
//! aims at test-range footage rather than a cartoon: a white-hot flash with
//! a few short rays, a ring of dust racing out along the ground, then a
//! fireball that climbs a narrow stem into a wide cap rolling over itself,
//! briefly wrapped in a condensation shell and collar, cooling from a
//! glowing core to heavy smoke that leans with the wind while debris
//! streaks away.
//!
//! Every puff is shaded rather than outlined - a dark body, a shadow step
//! down and right, a lit side up and to the left, and while it still burns
//! a fire core that shrinks as it cools - all discs of whole 2 px blocks on
//! the field's grid, in the fire and smoke ramps; its light is stepped
//! glows. Time is continuous, so it animates every frame.
//!
//! No two clouds match: `Cloud::new` hashes the height, cap size, roll
//! speed, puff counts and pace from the blast's seed, and every puff hashes
//! its own place, size and cooling. The lean is the wind's, read where the
//! tank died (`pyro::smoke_lean`). Nothing draws RNG; `compose` is a pure
//! function of the cloud, its age and the wind, so it is tested headless
//! and `draw` only paints what it returns.

use crate::Position;
use crate::blast::hash_unit;
use crate::math::Color;
use crate::pyro::{self, Blocks, Puff, Shape, BLOCK, DUST, SMOKE};
use crate::tuning::tuning;
use std::f32::consts::{PI, TAU};

/// Fire from white hot to embers, brightest first: the effects ramp's
/// white, pale gold, gold, gold-orange, red, deep red and darkest red
/// (`pyro::FIRE`, top down).
const FIRE: [Color; 7] = [pyro::FIRE[7], pyro::FIRE[6], pyro::FIRE[5], pyro::FIRE[4], pyro::FIRE[3], pyro::FIRE[2], pyro::FIRE[1]];

/// Condensation: the pale shell and collar the shock leaves in the air.
const VAPOUR: Color = SMOKE[6];

/// The dust the shock lifts off the ground as it races out: the pale sand
/// front and its darker echo.
const SHOCK_DUST: [Color; 2] = [DUST[4], DUST[2]];

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
    /// This cloud's own sideways drift per px of height, signed: a small
    /// hashed nudge on top of the wind's.
    wind: f32,
    /// How fast (rad/s) the cap rolls over itself.
    roll: f32,
    /// Vertical squash of everything that lies in the ground plane (the
    /// cap's ring, the shock ring, the dust) - the tilt of the top-down view.
    squash: f32,
    stem_width: f32,
    /// Puff spacings per second the stem's puffs travel up it.
    flow: f32,
    /// Phase of the stem's wobble.
    wobble: f32,
    /// Where up the stem (fraction of its height) the collar forms.
    collar: f32,
    cap_puffs: u32,
    dome_puffs: u32,
    skirt_puffs: u32,
    debris: u32,
}

use pyro::{alpha as fade, ease_out};

/// How much of a puff is left once it starts to dissolve at `start`
/// (a fraction of the cloud's life): 1 until then, 0 a short while after.
fn dissolve(p: f32, start: f32) -> f32 {
    1.0 - ((p - start) / 0.14).clamp(0.0, 1.0)
}

/// A shaded puff by `heat` (1 white hot, 0 burnt out) and `ash` (0 soot,
/// 1 pale) - the effects language's shading (`pyro::shade`) - with one
/// case of its own: `underlit` is a smoke puff on the cap's underside,
/// kept dark under the warm glow the cap's belly throws.
fn puff(pos: Position, radius: f32, heat: f32, ash: f32, sunlit: bool, underlit: bool) -> Puff {
    let (body, shadow, lit, core) = pyro::shade(heat, ash, sunlit && !underlit);
    if underlit && heat <= pyro::FIREBALL {
        return Puff { pos, radius, body: SMOKE[1], shadow: Some(SMOKE[0]), lit: None, core, cover: 1.0 };
    }
    Puff { pos, radius, body, shadow, lit, core, cover: 1.0 }
}

impl Cloud {
    /// The cloud for a blast with this `seed`, sized by its `scale`.
    pub fn new(seed: u32, scale: f32) -> Self {
        let t = tuning();
        let u = |k: u32| hash_unit(seed, k);
        let cap = t.mushroom_cap_px * scale * (0.85 + 0.4 * u(3));
        Cloud {
            seed,
            seconds: t.mushroom_seconds * (0.85 + 0.3 * u(1)),
            height: t.mushroom_height_px * scale * (0.8 + 0.45 * u(2)),
            cap,
            wind: (u(4) - 0.5) * 0.1,
            roll: 1.2 + 1.6 * u(5),
            squash: 0.3 + 0.14 * u(10),
            stem_width: cap * (0.14 + 0.07 * u(11)),
            flow: 1.5 + 2.0 * u(12),
            wobble: u(13) * TAU,
            collar: 0.45 + 0.2 * u(16),
            cap_puffs: 22 + (u(6) * 12.0) as u32,
            dome_puffs: 2 + (u(15) * 3.0) as u32,
            skirt_puffs: 16 + (u(8) * 10.0) as u32,
            debris: 10 + (u(9) * 10.0) as u32,
        }
    }

    pub fn done(&self, time: f32) -> bool {
        time >= self.seconds
    }

    fn u(&self, k: u32) -> f32 {
        hash_unit(self.seed, k)
    }

    /// Everything to paint `time` seconds after the kill at `base`, back
    /// to front, the smoke leaning `wind` px sideways per px it climbs
    /// (`pyro::smoke_lean`).
    pub fn compose(&self, base: Position, time: f32, wind: f32) -> Vec<Shape> {
        let d = self.seconds;
        let t = time.clamp(0.0, d);
        let p = t / d;
        let fire_life = 0.3 * d;

        // The head: climbs fast, then keeps drifting up; spreads as it goes.
        let h = self.height * (ease_out(t / (0.36 * d)) + 0.1 * p);
        let lean = (self.wind + wind) * (0.6 + 0.8 * p);
        let head = Position::new(base.x + lean * h, base.y - h);
        let cap = self.cap * (0.3 + 0.7 * ease_out(t / (0.42 * d))) * (1.0 + 0.2 * p);
        let ring = cap * 0.74;
        let tube = cap * 0.3;

        let mut out = Vec::new();
        let mut glows = Vec::new();

        // The shock ring, on the ground under everything: a thin front of
        // pale dust with a darker echo behind it.
        let shock = 0.5;
        if t < shock {
            let k = t / shock;
            let a = (1.0 - k).powf(1.5);
            let reach = self.cap * 3.4 * ease_out(k);
            self.ellipse(&mut out, base, reach, self.squash, 0.0, TAU, 1, fade(SHOCK_DUST[0], a));
            self.ellipse(&mut out, base, reach * 0.82, self.squash, 0.0, TAU, 2, fade(SHOCK_DUST[1], a * 0.6));
        }

        // Dust the shock lifts off the ground, rolling out behind the ring.
        let mut skirt_front = Vec::new();
        if t > 0.05 {
            for j in 0..self.skirt_puffs {
                let s = 600 + j * 4;
                let a = TAU * j as f32 / self.skirt_puffs as f32 + self.u(s) * 0.4 + t * 0.1;
                let reach = self.cap * 1.25 * ease_out(t / (0.55 * d)) * (0.75 + 0.5 * self.u(s + 1));
                let r = self.cap * 0.22 * (0.7 + 0.6 * self.u(s + 2)) * (1.0 - 0.3 * p) * dissolve(p, 0.3 + 0.35 * self.u(s + 3));
                if r < BLOCK {
                    continue;
                }
                let pos = Position::new(base.x + a.cos() * reach, base.y + 4.0 + a.sin() * reach * self.squash);
                let mut p = puff(pos, r, 0.0, 0.5 + 0.4 * p, true, false);
                p.core = None;
                let p = Shape::Puff(p);
                if a.sin() < 0.0 { out.push(p) } else { skirt_front.push(p) }
            }
        }

        // The condensation collar around the stem: a flat pale ring that
        // forms as the stem rises through it and thins away.
        let collar_env = self.envelope(p, 0.18, 0.26, 0.45);
        let collar_at = Position::new(base.x + lean * h * self.collar, base.y - h * self.collar);
        let collar_r = self.stem_width * 3.2 + cap * 0.3;
        if collar_env > 0.0 {
            self.ellipse(&mut out, collar_at, collar_r, self.squash, PI, TAU, 2, fade(VAPOUR, 0.3 * collar_env));
        }

        // The stem: puffs ride up a conveyor, fed from the ground, flared
        // at the foot; it breaks up from the bottom once the fire is out.
        let cutoff = ((p - 0.5) / 0.3).clamp(0.0, 1.0);
        let top = (h - tube * 0.4).max(1.0);
        let spacing = (self.stem_width * 0.55).max(2.0);
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
            let wob = (frac * 9.0 + t * 4.0 + self.wobble).sin() * 1.5;
            let pos = Position::new(base.x + lean * rise + wob, base.y - rise);
            let feed = (frac / 0.12).clamp(0.0, 1.0);
            let flare = 1.0 + 0.8 * (1.0 - frac).powi(3);
            let edge = if cutoff > 0.0 { ((frac - cutoff) / 0.15).clamp(0.0, 1.0) } else { 1.0 };
            let r = self.stem_width * flare * feed * (1.0 - 0.35 * p) * edge;
            if r < BLOCK {
                continue;
            }
            let hot = fire_life * (1.2 + 0.9 * (1.0 - frac));
            let heat = (1.0 - t / hot).max(0.0);
            let ash = 0.1 + ((t - hot) / (0.6 * d)).clamp(0.0, 1.0) * 0.5;
            if heat > 0.3 {
                glows.push(Shape::Glow { pos, radius: r * 2.0, color: fade(FIRE[2], 0.18 * heat) });
            }
            out.push(Shape::Puff(puff(pos, r, heat, ash, false, false)));
        }

        if collar_env > 0.0 {
            self.ellipse(&mut out, collar_at, collar_r, self.squash, 0.0, PI, 2, fade(VAPOUR, 0.45 * collar_env));
        }

        // The cap: a ring of puffs rolling over itself like a smoke ring -
        // out over the top, down the outside, back in underneath - seen
        // from above at the view's tilt. The underside keeps its fire and
        // then its glow longest; the top cools first and pales.
        let mut cap_back: Vec<(f32, Puff)> = Vec::new();
        let mut cap_front: Vec<(f32, Puff)> = Vec::new();
        for i in 0..self.cap_puffs {
            let s = 100 + i * 6;
            let az = TAU * i as f32 / self.cap_puffs as f32 + self.u(s) * 0.4;
            let roll = self.u(s + 1) * TAU - self.roll * t;
            let radial = ring + tube * roll.cos();
            let up = tube * roll.sin() * 0.6;
            let depth = az.sin();
            let pos = Position::new(head.x + radial * az.cos(), head.y - up + radial * depth * self.squash);
            let r = tube * (0.85 + 0.45 * self.u(s + 2)) * dissolve(p, 0.56 + 0.28 * self.u(s + 3));
            if r < BLOCK {
                continue;
            }
            let under = roll.sin() < 0.0;
            let hot = fire_life * (0.55 + 0.7 * self.u(s + 4)) * if under { 1.5 } else { 1.0 };
            let heat = (1.0 - t / hot).max(0.0);
            let ash = 0.15 + 0.85 * ((t - hot) / (0.6 * d)).clamp(0.0, 1.0) * (0.5 + 0.5 * (roll.sin() * 0.5 + 0.5));
            let underlit = under && t < hot * 1.35;
            if heat > 0.3 {
                glows.push(Shape::Glow { pos, radius: r * 1.8, color: fade(FIRE[2], 0.15 * heat) });
            }
            let p = puff(pos, r, heat, ash, roll.sin() > 0.2, underlit);
            if depth >= 0.0 { cap_front.push((depth, p)) } else { cap_back.push((depth, p)) }
        }
        // Back to front, so the near side of the ring covers the far side.
        let by_depth = |a: &(f32, Puff), b: &(f32, Puff)| a.0.total_cmp(&b.0);
        cap_back.sort_by(by_depth);
        cap_front.sort_by(by_depth);
        out.extend(cap_back.into_iter().map(|(_, p)| Shape::Puff(p)));

        // The belly still glowing from the fire under it: a translucent
        // warm band along the cap's underside, fading once the fire is out.
        let belly = 1.0 - ((t - fire_life) / (0.35 * d)).clamp(0.0, 1.0);
        if belly > 0.0 && t > 0.2 * fire_life {
            for i in 0..3 {
                let x = head.x + (i as f32 - 1.0) * ring * 0.55;
                glows.push(Shape::Glow { pos: Position::new(x, head.y + tube * 0.6), radius: cap * 0.42, color: fade(FIRE[4], 0.24 * belly) });
            }
        }

        // The dome the ring rolls around, bulging up out of its middle.
        for i in 0..self.dome_puffs {
            let s = 300 + i * 5;
            let pos = Position::new(
                head.x + (self.u(s) - 0.5) * cap * 0.8,
                head.y - tube * (0.3 + 0.4 * self.u(s + 1)) + (t * 2.0 + self.u(s + 2) * TAU).sin(),
            );
            let r = tube * (0.9 + 0.3 * self.u(s + 3)) * dissolve(p, 0.6 + 0.24 * self.u(s + 4));
            if r < BLOCK {
                continue;
            }
            let hot = fire_life * (0.45 + 0.4 * self.u(s + 4));
            let heat = (1.0 - t / hot).max(0.0);
            let ash = 0.3 + 0.7 * ((t - hot) / (0.5 * d)).clamp(0.0, 1.0);
            out.push(Shape::Puff(puff(pos, r, heat, ash, true, false)));
        }
        out.extend(cap_front.into_iter().map(|(_, p)| Shape::Puff(p)));
        out.extend(skirt_front);

        // The condensation shell: a pale arc over the cap that the shock
        // leaves in the air for a moment.
        let shell = self.envelope(p, 0.1, 0.18, 0.42);
        if shell > 0.0 {
            let rx = cap * 1.45;
            self.ellipse(&mut out, Position::new(head.x, head.y + tube * 0.4), rx, 0.62, PI * 1.08, PI * 1.92, 2, fade(VAPOUR, 0.5 * shell));
            self.ellipse(&mut out, Position::new(head.x, head.y + tube * 0.4), rx * 1.08, 0.62, PI * 1.15, PI * 1.85, 3, fade(VAPOUR, 0.3 * shell));
        }

        // Debris streaking away on ballistic arcs: hot fragments and dark
        // chunks, each drawn as a short trail behind where it is now.
        for e in 0..self.debris {
            let s = 800 + e * 5;
            let life = 0.5 + 0.9 * self.u(s);
            if t < 0.03 || t >= life {
                continue;
            }
            let a = -PI / 2.0 + (self.u(s + 1) - 0.5) * 2.6;
            let speed = (90.0 + 150.0 * self.u(s + 2)) * self.cap / 30.0;
            let at = |t: f32| Position::new(base.x + a.cos() * speed * t, base.y + a.sin() * speed * t + 120.0 * t * t);
            let k = t / life;
            let color = if self.u(s + 3) < 0.4 {
                fade(SMOKE[0], 1.0 - k * k)
            } else {
                fade(if k < 0.3 { FIRE[1] } else if k < 0.6 { FIRE[3] } else { FIRE[5] }, 1.0 - k * k)
            };
            self.segment(&mut out, at((t - 0.05).max(0.0)), at(t), color);
        }

        // The flash: a white-hot core swelling for a frame and a few short
        // rays, gone in a few frames, over a wide warm light.
        let flash = 0.14;
        if t < flash {
            let k = 1.0 - t / flash;
            glows.push(Shape::Glow { pos: base, radius: 26.0 + 40.0 * k, color: fade(FIRE[1], 0.55 * k) });
            if t < flash * 0.6 {
                let rays = 6 + (self.u(40) * 3.0) as u32;
                for ray in 0..rays {
                    let a = TAU * (ray as f32 + 0.5 * self.u(41 + ray)) / rays as f32;
                    let len = (26.0 + 30.0 * self.u(50 + ray)) * (0.5 + 0.5 * k);
                    let tip = Position::new(base.x + a.cos() * len, base.y + a.sin() * len * 0.8);
                    out.push(Shape::Line { from: base, to: tip, width: 2.0, head: FIRE[0], tail: fade(FIRE[2], 0.0) });
                }
            }
            out.push(Shape::Puff(Puff { pos: base, radius: 4.0 + 12.0 * k, body: FIRE[1], shadow: None, lit: None, core: Some((FIRE[0], 0.7)), cover: 1.0 }));
        }

        out.extend(glows);
        out
    }

    /// 0 before `start`, ramping to 1 by `peak`, back to 0 by `end`
    /// (fractions of the cloud's life), with a hashed nudge per cloud.
    fn envelope(&self, p: f32, start: f32, peak: f32, end: f32) -> f32 {
        let nudge = (self.u(30) - 0.5) * 0.06;
        let p = p - nudge;
        if p <= start || p >= end {
            0.0
        } else if p < peak {
            (p - start) / (peak - start)
        } else {
            1.0 - (p - peak) / (end - peak)
        }
    }

    /// Marks along an ellipse of radius `rx` (and `rx * squash` tall)
    /// from angle `from` to `to`, one block apart; `stride` 2 or more
    /// leaves gaps, which is how vapour reads thinner than the shock.
    #[allow(clippy::too_many_arguments)]
    fn ellipse(&self, out: &mut Vec<Shape>, center: Position, rx: f32, squash: f32, from: f32, to: f32, stride: u32, color: Color) {
        if rx < BLOCK || color.a == 0 {
            return;
        }
        let steps = ((to - from) * rx / BLOCK).ceil().max(1.0) as u32;
        for i in (0..=steps).step_by(stride.max(1) as usize) {
            let a = from + (to - from) * i as f32 / steps as f32;
            let pos = Position::new(center.x + a.cos() * rx, center.y + a.sin() * rx * squash);
            out.push(Shape::Mark { pos, size: 2, color });
        }
    }

    /// Marks along a straight line, one block apart.
    fn segment(&self, out: &mut Vec<Shape>, from: Position, to: Position, color: Color) {
        if color.a == 0 {
            return;
        }
        let len = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
        let steps = (len / BLOCK).ceil().max(1.0) as u32;
        for i in 0..=steps {
            let k = i as f32 / steps as f32;
            let pos = Position::new(from.x + (to.x - from.x) * k, from.y + (to.y - from.y) * k);
            out.push(Shape::Mark { pos, size: 2, color });
        }
    }
}

/// Paint the cloud of a kill at `base`, `time` seconds in, leaning with
/// `wind` (`compose`): the puffs and marks, then its light, which a live
/// round draws inside an additive blend and a CPU preview simply lays
/// over. Through `pyro::Blocks`, so the headless tests and previews paint
/// what the game does.
pub fn draw(c: &mut impl Blocks, cloud: &Cloud, base: Position, time: f32, wind: f32) {
    let shapes = cloud.compose(base, time, wind);
    pyro::draw(c, &shapes);
    pyro::draw_glows(c, &shapes, tuning().glow_bands.max(0) as u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud(seed: u32) -> Cloud {
        Cloud::new(seed, 1.0)
    }

    fn puffs(c: &Cloud, time: f32) -> Vec<Puff> {
        c.compose(Position::new(200.0, 200.0), time, 0.0)
            .into_iter()
            .filter_map(|s| if let Shape::Puff(p) = s { Some(p) } else { None })
            .collect()
    }

    fn top(c: &Cloud, time: f32) -> f32 {
        puffs(c, time).iter().map(|p| p.pos.y - p.radius).fold(f32::MAX, f32::min)
    }

    #[test]
    fn the_cloud_flashes_climbs_burns_out_and_is_gone_at_the_end() {
        let c = cloud(0xB0B5);
        assert!(puffs(&c, 0.0).iter().any(|p| p.core.is_some_and(|(core, _)| core == FIRE[0])), "the white-hot flash is up on the first frame");
        assert!(top(&c, c.seconds * 0.5) < top(&c, 0.2) - 30.0, "the cap climbs well above the hull");
        let burning = |time: f32| puffs(&c, time).iter().filter(|p| p.core.is_some()).count();
        assert!(burning(0.3) > burning(c.seconds * 0.8), "the fire burns out into smoke");
        assert!(puffs(&c, c.seconds * 0.999).is_empty(), "every puff has dissolved");
    }

    #[test]
    fn the_shock_ring_races_out_and_fades() {
        let c = cloud(3);
        let base = Position::new(200.0, 200.0);
        let reach = |time: f32| {
            c.compose(base, time, 0.0)
                .iter()
                .filter_map(|s| match s {
                    Shape::Mark { pos, color, .. } if color.r == SHOCK_DUST[0].r && color.b == SHOCK_DUST[0].b => Some((pos.x - base.x).abs()),
                    _ => None,
                })
                .fold(0.0, f32::max)
        };
        assert!(reach(0.4) > reach(0.16) + 20.0, "the ring spreads");
        assert_eq!(reach(0.6), 0.0, "and is gone within a beat");
    }

    #[test]
    fn no_two_clouds_look_alike_and_one_cloud_always_looks_the_same() {
        let base = Position::new(300.0, 150.0);
        assert_eq!(cloud(1).compose(base, 0.8, 0.0), cloud(1).compose(base, 0.8, 0.0));
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
        assert_ne!(c.compose(base, 1.0, 0.0), c.compose(base, 1.0 + 1.0 / 60.0, 0.0));
    }

    /// `cargo test --lib mushroom::tests::preview -- --ignored` writes
    /// target/mushroom_preview.png (six clouds over their life, doubled)
    /// and target/mushroom_frames/*.png (four clouds at 30 fps, for a GIF).
    #[test]
    #[ignore]
    #[cfg(feature = "render")] // write_png is raylib's PNG encoder
    fn preview() {
        let ground = Color::new(0x5e, 0x80, 0x3c, 255);
        let hull = Color::new(0x44, 0x44, 0x44, 255);
        let (cols, rows, cell_w, cell_h) = (14, 6, 150, 190);
        let mut c = crate::canvas::CpuCanvas::blank(cols * cell_w, rows * cell_h);
        c.clear(ground);
        for r in 0..rows {
            let cl = cloud(crate::blast::seed_for(Position::new(r as f32 * 64.0 + 32.0, 96.0)));
            for k in 0..cols {
                let t = cl.seconds * k as f32 / (cols - 1) as f32 * 0.97;
                let base = Position::new((k * cell_w + cell_w / 2) as f32, (r * cell_h + cell_h - 40) as f32);
                c.fill_rect(base.x as i32 - 14, base.y as i32 - 10, 28, 20, hull);
                draw(&mut c, &cl, base, t, 0.0);
            }
        }
        c.write_png(std::path::Path::new("target/mushroom_preview.png"), 2).unwrap();

        let dir = std::path::Path::new("target/mushroom_frames");
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let clouds: Vec<Cloud> = (0..4).map(|r| cloud(crate::blast::seed_for(Position::new(r as f32 * 64.0 + 32.0, 96.0)))).collect();
        let longest = clouds.iter().map(|c| c.seconds).fold(0.0, f32::max);
        for f in 0..((longest + 0.3) * 30.0) as usize {
            let t = f as f32 / 30.0;
            let mut c = crate::canvas::CpuCanvas::blank(4 * 160, 220);
            c.clear(ground);
            for (i, cl) in clouds.iter().enumerate() {
                let base = Position::new(80.0 + i as f32 * 160.0, 175.0);
                c.fill_rect(base.x as i32 - 14, base.y as i32 - 10, 28, 20, hull);
                if !cl.done(t) {
                    draw(&mut c, cl, base, t, 0.0);
                }
            }
            c.write_png(&dir.join(format!("{f:03}.png")), 2).unwrap();
        }
    }
}
