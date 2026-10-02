//! The sky without its shaders (docs/weather.md "Without shaders"),
//! headless: what a window whose GPU would not compile the weather's
//! passes draws instead of a clear sky. The sky is part of the round - the
//! enemies see less at night and in fog - so a player on an old device
//! must not see through it either.
//!
//! `render/weather.rs` multiplies the field by the light map with a blend
//! mode where the light pass would run, and draws the blocks composed here
//! where the ground and sky passes would: the snow lying on the ground
//! (`snow_cover`), then the fog, the blowing sand, the rain and the
//! falling snow (`air`). Each is the shader's own noise, amount and
//! clearing round every seat, worked out per `TILE_PX` tile rather than per
//! 2 px block and stepped hard rather than dithered, and a run of equal
//! tiles along a row is one block. The light's bands, the moonlit grey,
//! the dusk sun, the vignette, the heat haze, the lit fog round a lamp and
//! the ground's wet sheen, puddles and ice are the shaders' alone.

use std::sync::atomic::{AtomicBool, Ordering};

use super::{smoothstep, Look, Rgb};
use crate::math::{Color, Rectangle, Vec2};
use crate::tuning::Tuning;

/// The side of a fog, sand or snow-cover tile (px): four of the art's
/// 2 px blocks, so a bank's edge steps like drawn pixels, and a view is a
/// few thousand tiles.
pub const TILE_PX: f32 = 8.0;

/// Set once this process's weather shaders would not compile.
static SHADERS_MISSING: AtomicBool = AtomicBool::new(false);

/// Record that this window's GPU would not compile the weather's shaders
/// (`render::weather::WeatherFx::load`): every sky from then on is drawn
/// without them.
pub fn note_shaders_missing() {
    SHADERS_MISSING.store(true, Ordering::Relaxed);
}

/// Whether this process draws its skies without the shaders: they would
/// not compile, or `weather_without_shaders` asks for it. The dev
/// server's `weather` reply carries it as `without_shaders`.
pub fn without_shaders(t: &Tuning) -> bool {
    SHADERS_MISSING.load(Ordering::Relaxed) || t.weather_without_shaders
}

/// One block of the plain sky: a rectangle in world pixels on the 2 px
/// block grid, and its colour, drawn alpha-blended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub rect: Rectangle,
    pub color: Color,
}

/// One frame's air, as the renderer resolved it for the sky pass.
pub struct Air<'a> {
    pub look: &'a Look,
    /// The round clock, wrapped as the shaders take it.
    pub time: f32,
    /// The light the field is lit by where no lamp is, lightning's lift
    /// included: the fog, the sand, the rain and the snow are lit by it.
    pub ambient: Rgb,
    /// Every seat's tank: fog and sand thin round each.
    pub seats: &'a [Vec2],
    /// The sandstorm gust crossing the field: seconds since its front
    /// left, and the way it blows.
    pub gust: Option<(f32, Vec2)>,
}

/// The snow lying on the bare ground in `view` (world px): drawn over the
/// ground tileset and under the tread marks, so a tank leaves dark tracks
/// in it. The ground pass's patches - the same noise, more of it as
/// `snow_cover` rises, in three hard steps - in one flake colour, since
/// the ground's own relief under it is the pass's alone. Nothing under a
/// sky without snow.
pub fn snow_cover(look: &Look, view: Rectangle, t: &Tuning, out: &mut Vec<Block>) {
    let s = (look.snow * t.snow_cover).clamp(0.0, 1.0);
    if s <= 0.0 {
        return;
    }
    // The pass lightens a flake by the ground under it; a mid-bright
    // ground's flake for every tile.
    let flake = [0.92 * 0.97, 0.95 * 0.97, 0.97];
    tiles(view, out, |p| {
        let cover = smoothstep(0.62 - 0.5 * s, 0.8 - 0.5 * s, fbm(p.x * 0.035, p.y * 0.035));
        color(flake, steps(cover, 3.0) * 0.86)
    });
}

/// The air over the finished field in `view` (world px): fog, blowing
/// sand, rain and snow, in the sky pass's order, every block touching the
/// view. Nothing under a sky with none of them.
pub fn air(air: &Air, view: Rectangle, t: &Tuning, out: &mut Vec<Block>) {
    let start = out.len();
    fog(air, view, t, out);
    sand(air, view, t, out);
    rain(air, view, t, out);
    snow(air, view, out);
    // A streak or flake from a cell the view only grazes can fall wholly
    // outside it.
    let touching = |r: Rectangle| r.x < view.x + view.width && view.x < r.x + r.width && r.y < view.y + view.height && view.y < r.y + r.height;
    let kept: Vec<Block> = out.drain(start..).filter(|b| touching(b.rect)).collect();
    out.extend(kept);
}

/// Fog banks drifting over the field, thinned round every seat: the sky
/// pass's `fog` noise and amount in four hard steps.
fn fog(air: &Air, view: Rectangle, t: &Tuning, out: &mut Vec<Block>) {
    let fog = air.look.fog;
    if fog <= 0.0 {
        return;
    }
    let tint = [0.84 * air.ambient[0].min(1.25), 0.87 * air.ambient[1].min(1.25), 0.9 * air.ambient[2].min(1.25)];
    let (dx, dy) = (air.time * 0.02 * t.fog_drift_speed, air.time * 0.006 * t.fog_drift_speed);
    tiles(view, out, |p| {
        let (qx, qy) = (p.x * 0.0045 + dx, p.y * 0.0045 + dy);
        let n = fbm(qx, qy * 1.6);
        let a = (smoothstep(0.28, 0.72, n) * 0.7 + 0.22) * fog * sight_mask(p, air.seats, t.weather_clear_radius_px);
        color(tint, steps(a, 4.0).min(0.9))
    });
}

/// Blowing sand in five hard steps, thinned round every seat except where
/// a gust's wall of it passes, and the grains streaking along the wind:
/// the sky pass's `sand` and `grains`.
fn sand(air: &Air, view: Rectangle, t: &Tuning, out: &mut Vec<Block>) {
    let sand = air.look.sand;
    if sand <= 0.0 {
        return;
    }
    let shade = (lum(air.ambient) * 1.1).clamp(0.25, 1.2);
    let wind = t.sand_wind_speed;
    tiles(view, out, |p| {
        let g = gust_strength(p, air.gust, t);
        let (qx, qy) = (p.x * 0.004 - air.time * 0.5 * wind, p.y * 0.013);
        let n = fbm(qx, qy);
        let n2 = vnoise(qx * 3.0 - air.time * 1.4 * wind, qy * 3.0);
        let clear = sight_mask(p, air.seats, t.weather_clear_radius_px);
        let mut a = (0.3 + 0.5 * smoothstep(0.3, 0.75, n) + 0.15 * n2) * sand * (clear + (1.0 - clear) * g);
        a += g * (0.26 + 0.18 * n2) * (sand * 1.4).min(1.0);
        // The grain of the colour in two steps, so a run of tiles stays one
        // block.
        let k = (0.9 + 0.2 * steps(n2, 2.0)) * shade;
        color([0.84 * k, 0.66 * k, 0.42 * k], steps(a.min(0.9), 5.0))
    });
    let grain = color([0.93 * shade, 0.8 * shade, 0.58 * shade], 0.55);
    for i in 0..2 {
        let fi = i as f32;
        let (cw, ch) = (48.0 + fi * 16.0, 10.0 + fi * 4.0);
        let run = air.time * (340.0 + fi * 160.0) * wind;
        let long = 4.0 + fi * 4.0;
        // Grains stand still in a frame sliding downwind, each a short
        // streak in its cell, bobbing on a wave across the field.
        for cy in cells(view.y - 8.0, view.y + view.height + 8.0, ch) {
            for cx in cells(view.x - run - cw, view.x + view.width - run, cw) {
                let (x, y) = (cx as f32, cy as f32);
                let yo = (hash(x + 1.7, y + 1.7) * (ch / 2.0)).floor() * 2.0 + 1.0;
                let xo = hash(x + 5.3, y + 5.3) * cw * 0.5;
                let left = x * cw + xo + run;
                let at = Vec2::new(left + long / 2.0, y * ch + yo - ((left + long / 2.0) * 0.01 + air.time).sin() * 6.0);
                if hash(x + fi * 7.0, y + fi * 7.0) > sand * (0.6 + 0.9 * gust_strength(at, air.gust, t)) {
                    continue;
                }
                let (bx, by) = (snap(left), snap(at.y));
                out.push(Block { rect: Rectangle::new(bx, by, snap(left + long + 1.0) - bx, 2.0), color: grain });
            }
        }
    }
}

/// Rain in three depths: streaks one block wide slanting down the wind,
/// each a block per 2 px row and fading in toward its foot in two steps -
/// the sky pass's `rainLayer`, where every drop stands still in sheared
/// space and the whole pattern falls.
fn rain(air: &Air, view: Rectangle, t: &Tuning, out: &mut Vec<Block>) {
    let rain = air.look.rain;
    if rain <= 0.0 {
        return;
    }
    let lit = air.ambient.map(|l| (l * 1.2 + 0.05).min(1.4));
    let tint = [0.78 * lit[0], 0.85 * lit[1], 0.97 * lit[2]];
    let slant = t.rain_slant;
    let (top, bottom) = (view.y, view.y + view.height);
    // How far a streak's column leans, either way, across the view's
    // height.
    let (lean_lo, lean_hi) = ((top * slant).min(bottom * slant), (top * slant).max(bottom * slant));
    for i in 0..3 {
        let fi = i as f32;
        let (cw, ch) = (12.0 + fi * 6.0, 96.0 + fi * 40.0);
        let fall = air.time * t.rain_speed_px * (0.8 + fi * 0.35);
        let len = 8.0 + fi * 7.0;
        let strength = 0.22 + 0.16 * fi;
        let chance = rain * (0.55 + 0.25 * fi);
        // A streak starts up to half a cell below its cell's top.
        for cy in cells(top - fall - len - ch, bottom - fall, ch) {
            for cx in cells(view.x - lean_hi - 2.0, view.x + view.width - lean_lo + 2.0, cw) {
                let (x, y) = (cx as f32, cy as f32);
                if hash(x + fi * 17.0, y + fi * 17.0) > chance {
                    continue;
                }
                let column = x * cw + (hash(x + 3.1, y + 3.1) * (cw / 2.0 - 1.0)).floor() * 2.0 + 1.0;
                let head = y * ch + hash(x + 7.7, y + 7.7) * ch * 0.5 + fall;
                streak(out, column, head, len, slant, strength, tint);
            }
        }
    }
}

/// One rain streak from world row `head` down `len` px: on every 2 px row
/// the one block whose centre lies within a pixel of the slanted column,
/// joined into one block where rows share a column, the upper half at
/// half the foot's strength.
fn streak(out: &mut Vec<Block>, column: f32, head: f32, len: f32, slant: f32, strength: f32, tint: Rgb) {
    let mut row = (head / 2.0).floor() * 2.0;
    if row + 1.0 <= head {
        row += 2.0;
    }
    let mut open: Option<Block> = None;
    while row + 1.0 < head + len {
        let along = row + 1.0 - head;
        let alpha = strength * if along < len * 0.5 { 0.47 } else { 0.83 };
        let block = Block { rect: Rectangle::new(snap(column + (row + 1.0) * slant), row, 2.0, 2.0), color: color(tint, alpha) };
        open = match open {
            Some(mut b) if b.rect.x == block.rect.x && b.color == block.color => {
                b.rect.height += 2.0;
                Some(b)
            }
            Some(b) => {
                out.push(b);
                Some(block)
            }
            None => Some(block),
        };
        row += 2.0;
    }
    out.extend(open);
}

/// Snow in three depths, a flake per cell drifting as it falls - the sky
/// pass's `flakes`, the deepest a block across and the nearest two.
fn snow(air: &Air, view: Rectangle, out: &mut Vec<Block>) {
    let amount = air.look.snow * 0.55;
    if amount <= 0.0 {
        return;
    }
    let lit = air.ambient.map(|l| (l * 1.1 + 0.1).min(1.3));
    let tint = [0.96 * lit[0], 0.98 * lit[1], lit[2]];
    for i in 0..3 {
        let fi = i as f32;
        let cs = 26.0 + fi * 12.0;
        let (drift, fall) = (air.time * (8.0 + fi * 6.0), air.time * (26.0 + fi * 22.0));
        let size = if i == 2 { 4.0 } else { 2.0 };
        let flake = color(tint, 0.65 + 0.12 * fi);
        for cy in cells(view.y - fall - cs, view.y + view.height - fall + cs, cs) {
            for cx in cells(view.x - drift - cs, view.x + view.width - drift + cs, cs) {
                let (x, y) = (cx as f32, cy as f32);
                let h = hash(x + fi * 31.0, y + fi * 31.0);
                if h > amount {
                    continue;
                }
                let sway = (air.time * (1.1 + h) + h * 6.28).sin() * 5.0;
                let fx = ((x * cs + hash(x + 1.1, y + 1.1) * (cs - 8.0) + 4.0 + sway) / 2.0).floor() * 2.0 + 1.0;
                let fy = ((y * cs + hash(x + 2.2, y + 2.2) * (cs - 8.0) + 4.0) / 2.0).floor() * 2.0 + 1.0;
                let (wx, wy) = (fx + drift, fy + fall);
                let (bx, by) = if size > 2.0 { (snap(wx - 1.0), snap(wy - 1.0)) } else { (snap(wx), snap(wy)) };
                out.push(Block { rect: Rectangle::new(bx, by, size, size), color: flake });
            }
        }
    }
}

/// Every `TILE_PX` tile of the world's grid that `view` overlaps, row by
/// row, coloured by `paint` at its centre; a run of equal colours along a
/// row goes out as one block, a transparent tile as none.
fn tiles(view: Rectangle, out: &mut Vec<Block>, mut paint: impl FnMut(Vec2) -> Color) {
    let x0 = (view.x / TILE_PX).floor() as i32;
    let x1 = ((view.x + view.width) / TILE_PX).ceil() as i32;
    let y0 = (view.y / TILE_PX).floor() as i32;
    let y1 = ((view.y + view.height) / TILE_PX).ceil() as i32;
    for ty in y0..y1 {
        let y = ty as f32 * TILE_PX;
        let mut run: Option<(i32, Color)> = None;
        for tx in x0..=x1 {
            let next = (tx < x1).then(|| paint(Vec2::new((tx as f32 + 0.5) * TILE_PX, y + TILE_PX / 2.0))).filter(|c| c.a > 0);
            if run.is_some_and(|(_, c)| Some(c) == next) {
                continue;
            }
            if let Some((start, c)) = run {
                out.push(Block { rect: Rectangle::new(start as f32 * TILE_PX, y, (tx - start) as f32 * TILE_PX, TILE_PX), color: c });
            }
            run = next.map(|c| (tx, c));
        }
    }
}

/// The cells of side `side` along one axis that `lo..hi` touches.
fn cells(lo: f32, hi: f32, side: f32) -> std::ops::RangeInclusive<i32> {
    (lo / side).floor() as i32..=(hi / side).floor() as i32
}

/// `x` down onto the 2 px block grid.
fn snap(x: f32) -> f32 {
    (x / 2.0).floor() * 2.0
}

/// `x` in `levels` hard steps, to the nearest: the shaders' `band` without
/// its dither, which a tile is too coarse to carry.
fn steps(x: f32, levels: f32) -> f32 {
    (x * levels + 0.5).floor() / levels
}

/// 1 far from every seat's tank, easing down to a quarter beside one: the
/// sky pass's `sightMask`.
fn sight_mask(p: Vec2, seats: &[Vec2], radius: f32) -> f32 {
    if radius <= 0.0 {
        return 1.0;
    }
    let s = seats.iter().fold(1.0f32, |s, &at| s.min(smoothstep(radius * 0.45, radius, p.distance_to(at))));
    0.25 + 0.75 * s
}

/// How hard the gust blows at `p`, 0 to 1: the band `weather::Gust`
/// pushes the hulls with, a fast rise behind the front and a slow dying
/// away - the sky pass's `gustAt`.
fn gust_strength(p: Vec2, gust: Option<(f32, Vec2)>, t: &Tuning) -> f32 {
    let Some((since, dir)) = gust else {
        return 0.0;
    };
    let age = (since - (p.x * dir.x + p.y * dir.y) / t.sand_gust_front_speed.max(1.0)) / t.sand_gust_seconds.max(0.05);
    if !(0.0..1.0).contains(&age) {
        return 0.0;
    }
    smoothstep(0.0, 0.2, age) * (1.0 - smoothstep(0.35, 1.0, age))
}

fn lum(c: Rgb) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

fn color(rgb: Rgb, alpha: f32) -> Color {
    let c = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::new(c(rgb[0]), c(rgb[1]), c(rgb[2]), c(alpha))
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// The shaders' `hash`: a lattice point to 0..1, its inputs wrapped so a
/// far one keeps its precision.
fn hash(x: f32, y: f32) -> f32 {
    let wrap = |v: f32| fract(v.rem_euclid(289.0) * 0.1031);
    let (mut a, mut b, mut c) = (wrap(x), wrap(y), wrap(x));
    let d = a * (b + 33.33) + b * (c + 33.33) + c * (a + 33.33);
    a += d;
    b += d;
    c += d;
    fract((a + b) * c)
}

/// The shaders' `vnoise`: value noise, smoothly blended between lattice
/// points.
fn vnoise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let lerp = |p: f32, q: f32, k: f32| p + (q - p) * k;
    let top = lerp(hash(ix, iy), hash(ix + 1.0, iy), ux);
    let bottom = lerp(hash(ix, iy + 1.0), hash(ix + 1.0, iy + 1.0), ux);
    lerp(top, bottom, uy)
}

/// The shaders' `fbm`: four octaves of `vnoise`.
fn fbm(mut x: f32, mut y: f32) -> f32 {
    let (mut sum, mut amp) = (0.0, 0.5);
    for _ in 0..4 {
        sum += amp * vnoise(x, y);
        x = x * 2.03 + 17.1;
        y = y * 2.03 + 9.3;
        amp *= 0.5;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Weather;

    const VIEW: Rectangle = Rectangle { x: 0.0, y: 0.0, width: 1088.0, height: 544.0 };

    fn compose(weather: Weather, seats: &[Vec2], time: f32) -> (Vec<Block>, Vec<Block>) {
        let t = Tuning::DEFAULT;
        let look = Look::of(weather, &t);
        let air_now = Air { look: &look, time, ambient: look.ambient, seats, gust: None };
        let (mut ground, mut sky) = (Vec::new(), Vec::new());
        snow_cover(&look, VIEW, &t, &mut ground);
        air(&air_now, VIEW, &t, &mut sky);
        (ground, sky)
    }

    fn grows(r: Rectangle, by: f32) -> Rectangle {
        Rectangle::new(r.x - by, r.y - by, r.width + 2.0 * by, r.height + 2.0 * by)
    }

    fn overlaps(a: Rectangle, b: Rectangle) -> bool {
        a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
    }

    #[test]
    fn a_sky_with_no_air_composes_no_blocks() {
        for weather in [Weather::Clear, Weather::Night, Weather::Dusk] {
            let (ground, sky) = compose(weather, &[], 3.0);
            assert!(ground.is_empty() && sky.is_empty(), "{weather:?}: {} + {} blocks", ground.len(), sky.len());
        }
    }

    #[test]
    fn every_sky_with_air_composes_some() {
        for weather in [Weather::Rain, Weather::Storm, Weather::Fog, Weather::Sandstorm, Weather::Snow] {
            let (_, sky) = compose(weather, &[], 3.0);
            assert!(!sky.is_empty(), "{weather:?} drew no air");
        }
        let (ground, _) = compose(Weather::Snow, &[], 3.0);
        assert!(!ground.is_empty(), "the snow lay nowhere");
    }

    #[test]
    fn every_block_is_on_the_block_grid_near_the_view() {
        let near = grows(VIEW, 64.0);
        for weather in Weather::ALL {
            let (ground, sky) = compose(weather, &[Vec2::new(300.0, 200.0)], 12.7);
            for b in ground.iter().chain(&sky) {
                let r = b.rect;
                for v in [r.x, r.y, r.width, r.height] {
                    assert_eq!(v.rem_euclid(2.0), 0.0, "{weather:?}: {r:?} is off the 2 px grid");
                }
                assert!(r.width > 0.0 && r.height > 0.0, "{weather:?}: empty {r:?}");
                assert!(overlaps(r, near), "{weather:?}: {r:?} is nowhere near the view");
                assert!(b.color.a > 0, "{weather:?}: a transparent block at {r:?}");
            }
        }
    }

    #[test]
    fn the_same_frame_composes_the_same_blocks() {
        for weather in Weather::ALL {
            assert_eq!(compose(weather, &[Vec2::new(500.0, 260.0)], 41.25), compose(weather, &[Vec2::new(500.0, 260.0)], 41.25), "{weather:?}");
        }
    }

    #[test]
    fn the_air_moves_with_the_clock() {
        for weather in [Weather::Rain, Weather::Fog, Weather::Sandstorm, Weather::Snow] {
            assert_ne!(compose(weather, &[], 10.0).1, compose(weather, &[], 10.5).1, "{weather:?} stood still");
        }
    }

    #[test]
    fn fog_thins_round_every_seat() {
        let seat = Vec2::new(544.0, 272.0);
        let (_, sky) = compose(Weather::Fog, &[seat], 5.0);
        // The fog's opacity at a point: the block covering it, or none.
        let at = |p: Vec2| sky.iter().find(|b| b.rect.contains(p)).map_or(0, |b| b.color.a);
        let mean = |points: &[Vec2]| points.iter().map(|&p| at(p) as f32).sum::<f32>() / points.len() as f32;
        let ring = |r: f32| -> Vec<Vec2> { (0..32).map(|i| { let a = i as f32 / 32.0 * std::f32::consts::TAU; seat + Vec2::new(a.cos() * r, a.sin() * r) }).collect() };
        let (beside, far) = (mean(&ring(20.0)), mean(&ring(240.0)));
        assert!(beside < far * 0.5, "fog beside the seat {beside} vs far {far}");
    }

    #[test]
    fn the_fog_steps_in_fours() {
        let (_, sky) = compose(Weather::Fog, &[], 5.0);
        let mut alphas: Vec<u8> = sky.iter().map(|b| b.color.a).collect();
        alphas.sort_unstable();
        alphas.dedup();
        assert!(alphas.len() <= 4, "fog in {} steps: {alphas:?}", alphas.len());
    }

    #[test]
    fn a_gust_thickens_the_sand_it_crosses() {
        let t = Tuning::DEFAULT;
        let look = Look::of(Weather::Sandstorm, &t);
        let seats = [Vec2::new(544.0, 272.0)];
        let opacity = |gust: Option<(f32, Vec2)>| {
            let mut out = Vec::new();
            fog_free_sand(&look, &seats, gust, &t, &mut out);
            out.iter().map(|b| b.color.a as f32 * b.rect.width * b.rect.height).sum::<f32>()
        };
        // A front that left the west edge half a gust ago is blowing over
        // the seat: the clearing round it fills.
        let blowing = Some((544.0 / t.sand_gust_front_speed + t.sand_gust_seconds * 0.3, Vec2::new(1.0, 0.0)));
        assert!(opacity(blowing) > opacity(None) * 1.05, "{} vs {}", opacity(blowing), opacity(None));
    }

    /// The sand alone, at a fixed clock.
    fn fog_free_sand(look: &Look, seats: &[Vec2], gust: Option<(f32, Vec2)>, t: &Tuning, out: &mut Vec<Block>) {
        sand(&Air { look, time: 8.0, ambient: look.ambient, seats, gust }, VIEW, t, out);
    }

    #[test]
    fn a_run_of_equal_tiles_is_one_block() {
        let mut out = Vec::new();
        tiles(Rectangle::new(0.0, 0.0, 64.0, 16.0), &mut out, |_| Color::new(1, 2, 3, 200));
        assert_eq!(out, vec![
            Block { rect: Rectangle::new(0.0, 0.0, 64.0, 8.0), color: Color::new(1, 2, 3, 200) },
            Block { rect: Rectangle::new(0.0, 8.0, 64.0, 8.0), color: Color::new(1, 2, 3, 200) },
        ]);
        out.clear();
        tiles(Rectangle::new(0.0, 0.0, 32.0, 8.0), &mut out, |p| Color::new(9, 9, 9, if p.x < 16.0 { 0 } else { 99 }));
        assert_eq!(out, vec![Block { rect: Rectangle::new(16.0, 0.0, 16.0, 8.0), color: Color::new(9, 9, 9, 99) }]);
    }

    #[test]
    fn the_knob_draws_without_shaders() {
        let mut t = Tuning::DEFAULT;
        t.weather_without_shaders = true;
        assert!(without_shaders(&t));
    }

    #[test]
    fn the_noise_stays_in_its_range() {
        for i in 0..2000 {
            let (x, y) = (i as f32 * 7.31 - 900.0, i as f32 * -3.17 + 40_000.0);
            let (h, v, f) = (hash(x, y), vnoise(x * 0.01, y * 0.01), fbm(x * 0.01, y * 0.01));
            assert!((0.0..1.0).contains(&h) && (0.0..=1.0).contains(&v) && (0.0..1.0).contains(&f), "{h} {v} {f} at {x},{y}");
        }
    }
}
