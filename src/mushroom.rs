//! The mushroom cloud a dying tank goes up in (docs/mushroom-cloud.md),
//! composed at draw time rather than played from a sheet. The look aims at
//! test-range footage rather than a cartoon: a white-cyan flash with a lens
//! streak, electric arcs crackling off the hull, a thin ionised shock ring
//! racing out along the ground, then a fireball that climbs a narrow stem
//! into a wide cap rolling over itself, briefly wrapped in a condensation
//! shell and collar, cooling from a glowing core to heavy smoke while
//! debris streaks away.
//!
//! Every puff is shaded rather than outlined - a dark body, a lit side up
//! and to the left, and while it still burns a fire core that shrinks as
//! it cools - all discs of whole 2 px blocks, so it sits on the same grid
//! as the sprites. Time is continuous, so it animates every frame.
//!
//! No two clouds match: `Cloud::new` hashes the height, cap size, lean,
//! roll speed, puff counts and pace from the blast's seed, and every puff
//! hashes its own place, size and cooling. Nothing draws RNG; `compose`
//! is a pure function of the cloud and its age, so it is tested headless
//! and `draw` only paints what it returns.

use crate::Position;
use crate::blast::hash_unit;
use crate::canvas::Canvas;
use crate::math::Color;
use crate::tuning::tuning;
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

/// Ionised light - the flash, the shock ring, the arcs: white, pale cyan,
/// electric blue. Deliberately off the palette, like the plasma sheet:
/// the one part of the cloud that is neither fire nor smoke.
const ION: [Color; 3] = [
    Color::new(0xFF, 0xFF, 0xFF, 255),
    Color::new(0xC8, 0xF0, 0xFF, 255),
    Color::new(0x6C, 0xC8, 0xFF, 255),
];

/// Condensation: the pale shell and collar the shock leaves in the air.
const VAPOUR: Color = Color::new(0xE6, 0xEE, 0xF2, 255);

/// The block everything is built from: the 2 screen px one sprite pixel
/// covers.
const BLOCK: f32 = 2.0;

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
    arcs: u32,
    debris: u32,
}

/// One shaded disc: `body`, then `lit` - a smaller disc up and left in a
/// lighter step - then `core`, the fire still burning inside, as a colour
/// and a fraction of the radius, a little below centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puff {
    pub pos: Position,
    pub radius: f32,
    pub body: Color,
    pub lit: Option<Color>,
    pub core: Option<(Color, f32)>,
}

/// What `compose` hands `draw`, in painting order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Puff(Puff),
    /// A single block (`size` 2) or a 2 x 2 of them (`size` 4): ring
    /// points, arcs, debris streaks, the lens streak.
    Mark { pos: Position, size: i32, color: Color },
    /// A translucent disc: bloom over whatever still burns.
    Glow { pos: Position, radius: f32, color: Color },
}

fn ease_out(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x) * (1.0 - x) * (1.0 - x)
}

/// `c` at opacity `a` (0..1), stepped to eighths like the particle layer's
/// alpha, so fades read as pixel-art steps rather than a smooth ramp.
fn fade(c: Color, a: f32) -> Color {
    let a = ((a.clamp(0.0, 1.0) * 8.0).round() / 8.0 * 255.0) as u8;
    Color::new(c.r, c.g, c.b, a)
}

/// How much of a puff is left once it starts to dissolve at `start`
/// (a fraction of the cloud's life): 1 until then, 0 a short while after.
fn dissolve(p: f32, start: f32) -> f32 {
    1.0 - ((p - start) / 0.14).clamp(0.0, 1.0)
}

/// Heat above this and a puff is still part of the fireball; below it the
/// puff is smoke with a fire core dying inside it.
const FIREBALL: f32 = 0.55;

/// The shading of a puff. `heat` 1 is white hot, 0 burnt out; `ash` 0 is
/// soot, 1 pale; `sunlit` gives a smoke puff its lit side; `underlit`
/// is a smoke puff on the cap's underside, kept dark under the warm glow
/// the cap's belly throws (drawn separately, see `compose`).
fn shade(heat: f32, ash: f32, sunlit: bool, underlit: bool) -> (Color, Option<Color>, Option<(Color, f32)>) {
    if heat > FIREBALL {
        let q = (heat - FIREBALL) / (1.0 - FIREBALL);
        let body = if q > 0.66 { FIRE[1] } else if q > 0.33 { FIRE[2] } else { FIRE[3] };
        let core = if q > 0.5 { FIRE[0] } else { FIRE[1] };
        return (body, None, Some((core, 0.45 + 0.3 * q)));
    }
    let s = 1 + ((ash.clamp(0.0, 1.0) * 3.0) as usize).min(2);
    let body = if underlit { SMOKE[1] } else { SMOKE[s] };
    let lit = (sunlit && !underlit).then_some(SMOKE[s + 1]);
    let core = (heat > 0.0).then(|| (if heat > 0.3 { FIRE[3] } else { FIRE[4] }, 0.2 + 0.5 * heat / FIREBALL));
    (body, lit, core)
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
            wind: (u(4) - 0.5) * 0.4,
            roll: 1.2 + 1.6 * u(5),
            squash: 0.3 + 0.14 * u(10),
            stem_width: cap * (0.14 + 0.07 * u(11)),
            flow: 1.5 + 2.0 * u(12),
            wobble: u(13) * TAU,
            collar: 0.45 + 0.2 * u(16),
            cap_puffs: 22 + (u(6) * 12.0) as u32,
            dome_puffs: 2 + (u(15) * 3.0) as u32,
            skirt_puffs: 16 + (u(8) * 10.0) as u32,
            arcs: 3 + (u(17) * 3.0) as u32,
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
    /// to front.
    pub fn compose(&self, base: Position, time: f32) -> Vec<Shape> {
        let d = self.seconds;
        let t = time.clamp(0.0, d);
        let p = t / d;
        let fire_life = 0.3 * d;

        // The head: climbs fast, then keeps drifting up; spreads as it goes.
        let h = self.height * (ease_out(t / (0.36 * d)) + 0.1 * p);
        let lean = self.wind * (0.6 + 0.8 * p);
        let head = Position::new(base.x + lean * h, base.y - h);
        let cap = self.cap * (0.3 + 0.7 * ease_out(t / (0.42 * d))) * (1.0 + 0.2 * p);
        let ring = cap * 0.74;
        let tube = cap * 0.3;

        let mut out = Vec::new();
        let mut glows = Vec::new();

        // The shock ring, on the ground under everything: a thin bright
        // front with a fainter echo behind it.
        let shock = 0.5;
        if t < shock {
            let k = t / shock;
            let a = (1.0 - k).powf(1.5);
            let reach = self.cap * 3.4 * ease_out(k);
            self.ellipse(&mut out, base, reach, self.squash, 0.0, TAU, 1, fade(ION[1], a));
            self.ellipse(&mut out, base, reach * 0.82, self.squash, 0.0, TAU, 2, fade(ION[2], a * 0.45));
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
                let (body, lit, _) = shade(0.0, 0.5 + 0.4 * p, true, false);
                let puff = Shape::Puff(Puff { pos, radius: r, body, lit, core: None });
                if a.sin() < 0.0 { out.push(puff) } else { skirt_front.push(puff) }
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
            let (body, lit, core) = shade(heat, ash, false, false);
            if heat > 0.3 {
                glows.push(Shape::Glow { pos, radius: r * 2.0, color: fade(FIRE[1], 0.18 * heat) });
            }
            out.push(Shape::Puff(Puff { pos, radius: r, body, lit, core }));
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
            let (body, lit, core) = shade(heat, ash, roll.sin() > 0.2, underlit);
            if heat > 0.3 {
                glows.push(Shape::Glow { pos, radius: r * 1.8, color: fade(FIRE[1], 0.15 * heat) });
            }
            let puff = Puff { pos, radius: r, body, lit, core };
            if depth >= 0.0 { cap_front.push((depth, puff)) } else { cap_back.push((depth, puff)) }
        }
        // Back to front, so the near side of the ring covers the far side.
        let by_depth = |a: &(f32, Puff), b: &(f32, Puff)| a.0.total_cmp(&b.0);
        cap_back.sort_by(by_depth);
        cap_front.sort_by(by_depth);
        out.extend(cap_back.into_iter().map(|(_, puff)| Shape::Puff(puff)));

        // The belly still glowing from the fire under it: a translucent
        // warm band along the cap's underside, fading once the fire is out.
        let belly = 1.0 - ((t - fire_life) / (0.35 * d)).clamp(0.0, 1.0);
        if belly > 0.0 && t > 0.2 * fire_life {
            for i in 0..3 {
                let x = head.x + (i as f32 - 1.0) * ring * 0.55;
                glows.push(Shape::Glow { pos: Position::new(x, head.y + tube * 0.6), radius: cap * 0.42, color: fade(FIRE[4], 0.3 * belly) });
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
            let (body, lit, core) = shade(heat, ash, true, false);
            out.push(Shape::Puff(Puff { pos, radius: r, body, lit, core }));
        }
        out.extend(cap_front.into_iter().map(|(_, puff)| Shape::Puff(puff)));
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
                fade(if k < 0.3 { FIRE[1] } else if k < 0.6 { FIRE[2] } else { FIRE[4] }, 1.0 - k * k)
            };
            self.segment(&mut out, at((t - 0.05).max(0.0)), at(t), color);
        }

        // Arcs crackling off the hull for the first moments, redrawn in a
        // new shape every tick so they flicker.
        let arc_time = 0.6;
        if t < arc_time {
            let tick = (t * 24.0) as u32;
            let a_fade = 1.0 - t / arc_time;
            for arc in 0..self.arcs {
                let key = 2000 + arc * 97 + tick * 7;
                if self.u(key) > 0.7 {
                    continue;
                }
                let mut dir = self.u(1500 + arc * 11 + tick / 3) * TAU;
                let mut from = Position::new(base.x + (self.u(key + 1) - 0.5) * 12.0, base.y + (self.u(key + 2) - 0.5) * 8.0);
                for seg in 0..(5 + (self.u(key + 3) * 4.0) as u32) {
                    dir += (self.u(key + 10 + seg) - 0.5) * 1.4;
                    let len = 5.0 + 5.0 * self.u(key + 30 + seg);
                    let to = Position::new(from.x + dir.cos() * len, from.y + dir.sin() * len * 0.8);
                    let color = if seg < 3 { ION[0] } else { ION[2] };
                    self.segment(&mut out, from, to, fade(color, a_fade));
                    from = to;
                }
                glows.push(Shape::Glow { pos: base, radius: 18.0, color: fade(ION[2], 0.12 * a_fade) });
            }
        }

        // The flash: a white-hot disc with a cyan halo and a horizontal
        // lens streak, gone in a few frames.
        let flash = 0.14;
        if t < flash {
            let k = 1.0 - t / flash;
            glows.push(Shape::Glow { pos: base, radius: 20.0 + 36.0 * k, color: fade(ION[1], 0.3 * k) });
            glows.push(Shape::Glow { pos: base, radius: 14.0 + 22.0 * k, color: fade(ION[0], 0.6 * k) });
            let long = 30.0 + 110.0 * k;
            for (y, half, color) in [(0.0, long, fade(ION[1], 0.8 * k)), (0.0, long * 0.45, ION[0]), (2.0, long * 0.5, fade(ION[2], 0.5 * k))] {
                self.segment(&mut glows, Position::new(base.x - half, base.y + y), Position::new(base.x + half, base.y + y), color);
            }
            out.push(Shape::Puff(Puff { pos: base, radius: 4.0 + 12.0 * k, body: ION[0], lit: None, core: None }));
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

/// Paint the cloud of a kill at `base`, `time` seconds in. Through a
/// `Canvas`, so the headless tests and previews paint what the game does.
pub fn draw(c: &mut impl Canvas, cloud: &Cloud, base: Position, time: f32) {
    for shape in cloud.compose(base, time) {
        match shape {
            Shape::Puff(puff) => {
                let r = puff.radius;
                block_disc(c, puff.pos, r, puff.body);
                if let Some(lit) = puff.lit {
                    block_disc(c, Position::new(puff.pos.x - r * 0.18, puff.pos.y - r * 0.24), r * 0.68, lit);
                }
                if let Some((core, frac)) = puff.core {
                    block_disc(c, Position::new(puff.pos.x, puff.pos.y + r * 0.12), r * frac, core);
                }
            }
            Shape::Mark { pos, size, color } => {
                let x = (pos.x / BLOCK).floor() * BLOCK;
                let y = (pos.y / BLOCK).floor() * BLOCK;
                c.fill_rect(x as i32, y as i32, size, size, color);
            }
            Shape::Glow { pos, radius, color } => block_disc(c, pos, radius, color),
        }
    }
}

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

    fn puffs(c: &Cloud, time: f32) -> Vec<Puff> {
        c.compose(Position::new(200.0, 200.0), time)
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
        assert!(puffs(&c, 0.0).iter().any(|p| p.body == ION[0]), "the flash is up on the first frame");
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
            c.compose(base, time)
                .iter()
                .filter_map(|s| match s {
                    Shape::Mark { pos, color, .. } if color.r == ION[1].r && color.b == ION[1].b => Some((pos.x - base.x).abs()),
                    _ => None,
                })
                .fold(0.0, f32::max)
        };
        // After the flash, whose lens streak shares the ring's colour.
        assert!(reach(0.4) > reach(0.16) + 20.0, "the ring spreads");
        assert_eq!(reach(0.6), 0.0, "and is gone within a beat");
    }

    #[test]
    fn no_two_clouds_look_alike_and_one_cloud_always_looks_the_same() {
        let base = Position::new(300.0, 150.0);
        assert_eq!(cloud(1).compose(base, 0.8), cloud(1).compose(base, 0.8));
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
                draw(&mut c, &cl, base, t);
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
                    draw(&mut c, cl, base, t);
                }
            }
            c.write_png(&dir.join(format!("{f:03}.png")), 2).unwrap();
        }
    }
}
