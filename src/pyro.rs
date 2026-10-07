//! The effects language (docs/effects.md): the one set of colours and the
//! one grid every explosion, hit, fire, smoke plume and damage mark is
//! drawn in, so a fireball, a shell's burst and a burning hull read as the
//! same fire, and all of it as part of the same pixel-art field.
//!
//! Three rules, each a function here:
//!
//! - **Whole 2 px blocks.** Every sprite in the game covers the field in
//!   2 px blocks (tanks draw a 32 px tile at scale 2, the wall sheet bakes
//!   the same chunkiness in), so everything that lingers on screen does
//!   too: [`block_disc`], [`dither_disc`], [`block_line`] and the marks of
//!   [`Shape`] all lie on the field's even grid.
//! - **Colours from ramps.** Fire, smoke, dust and char are steps of the
//!   Puny palette ([`FIRE`], [`SMOKE`], [`DUST`], [`CHAR`]); only energy
//!   (plasma, the laser, the tesla, ooze) keeps its own deliberately
//!   off-palette ramps, as the plasma sheet always has. A colour is a step,
//!   never a blend between steps; a fade is a dissolve through the Bayer
//!   pattern ([`bayer`]) down to half, then the kept blocks fading in
//!   eighths ([`dither_disc`]) - and smoke mostly breaks up by shrinking.
//! - **Light in steps.** A glow is a few flat bands of light, the band
//!   edges dithered across the middle third of each step ([`glow`]) -
//!   the same rule the weather's light pass draws the night with.
//!
//! What moves too fast or vanishes too soon to be looked at - a tracer, a
//! spark in flight, the first frames of a flash - is still drawn in blocks
//! and ramp colours, but may be bright and brief rather than shaded.
//!
//! Everything here is headless and pure: painters take a [`Blocks`] (any
//! `Canvas`, or the raylib handle through `render::pyro::Rl`), so the
//! tests paint what the game does.

use crate::canvas::Canvas;
use crate::math::Color;
use crate::Position;

/// The block everything is built from: the 2 screen px one sprite pixel
/// covers.
pub const BLOCK: f32 = 2.0;

const B: i32 = 2;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::new(r, g, b, 255)
}

/// Fire, from dying embers to white heat: the palette's red and gold steps
/// (RED_DARKEST, RED_DK, RED_DEEP, RED_MD, GOLD_MD, GOLD_BRIGHT), then
/// FIRE_PALE - the one warm step the palette gained for the core of a
/// blast (`tools/punypalette.py`) - and white.
pub const FIRE: [Color; 8] = [
    rgb(0x4A, 0x22, 0x21),
    rgb(0x81, 0x2F, 0x27),
    rgb(0x9C, 0x35, 0x27),
    rgb(0xE4, 0x42, 0x19),
    rgb(0xDC, 0x9C, 0x4A),
    rgb(0xEE, 0xA3, 0x43),
    rgb(0xFF, 0xE2, 0xA0),
    rgb(0xFF, 0xFF, 0xFF),
];

/// Smoke, from soot to pale ash: the palette's greys (BLACK,
/// STONE_DARKEST, STONE_SHADE, STONE_DK, STONE_MD, STONE_LT, STONE_HI).
pub const SMOKE: [Color; 7] = [
    rgb(0x25, 0x25, 0x25),
    rgb(0x37, 0x37, 0x37),
    rgb(0x5A, 0x5A, 0x5A),
    rgb(0x7E, 0x7E, 0x7E),
    rgb(0x9E, 0x9E, 0x96),
    rgb(0xC1, 0xC1, 0xC1),
    rgb(0xDA, 0xDA, 0xDA),
];

/// Dust and earth thrown up by a blast, dark to light (SAND_DK, WOOD_ASH,
/// SAND_MD, SAND_LT, SAND_PALE).
pub const DUST: [Color; 5] = [
    rgb(0x67, 0x51, 0x2A),
    rgb(0x73, 0x62, 0x4D),
    rgb(0xB7, 0xA2, 0x48),
    rgb(0xC9, 0xB2, 0x66),
    rgb(0xD2, 0xBA, 0x6B),
];

/// Char and soot on a hull or the ground: black, the darkest grey, the
/// darkest red and rust (BLACK, STONE_DARKEST, RED_DARKEST, RUST_DK).
pub const CHAR: [Color; 4] = [rgb(0x25, 0x25, 0x25), rgb(0x37, 0x37, 0x37), rgb(0x4A, 0x22, 0x21), rgb(0x59, 0x34, 0x1F)];

/// Energy ramps, dark to white-hot. Off the palette on purpose, like the
/// plasma sheet and the portals: the one light in the game that is not
/// fire.
/// The plasma bolts' ramps are their orbs' own colours
/// (`PlasmaVariant::orb_colors`), with the glint colour between.
pub const PLASMA_TEAL: [Color; 5] = [rgb(0x0A, 0x4E, 0x62), rgb(0x18, 0xA4, 0xB0), rgb(0x28, 0xDC, 0xC8), rgb(0x6E, 0xEC, 0xDE), rgb(0xEC, 0xFF, 0xFA)];
pub const PLASMA_PURPLE: [Color; 5] = [rgb(0x3A, 0x16, 0x68), rgb(0x80, 0x3C, 0xCC), rgb(0xB0, 0x6A, 0xF0), rgb(0xC4, 0x90, 0xFF), rgb(0xF8, 0xEA, 0xFF)];
/// A rainbow shield's scatter: its violet and white.
pub const SHIELD: [Color; 3] = [rgb(0x6A, 0x3C, 0xC8), rgb(0xAA, 0x78, 0xFF), rgb(0xFF, 0xFF, 0xFF)];
pub const TESLA: [Color; 5] = [rgb(0x4A, 0x26, 0x90), rgb(0x8C, 0x40, 0xFF), rgb(0x9A, 0x66, 0xFF), rgb(0xCA, 0xA6, 0xFF), rgb(0xFF, 0xFF, 0xFF)];
pub const LASER_RED: [Color; 5] = [rgb(0x6A, 0x14, 0x10), rgb(0xC8, 0x28, 0x1E), rgb(0xFF, 0x32, 0x28), rgb(0xFF, 0xAA, 0x96), rgb(0xFF, 0xFF, 0xFF)];
pub const LASER_BLUE: [Color; 5] = [rgb(0x10, 0x30, 0x6A), rgb(0x28, 0x60, 0xD0), rgb(0x28, 0x70, 0xFF), rgb(0x96, 0xC8, 0xFF), rgb(0xFF, 0xFF, 0xFF)];

/// Ooze, dark to wet highlight (the bio slush's own acid lime, `tower::OOZE_*`).
pub const OOZE: [Color; 4] = [crate::tower::OOZE_DK, crate::tower::OOZE_MD, crate::tower::OOZE_LT, crate::tower::OOZE_HI];

/// The dust a tile throws when it is hit or comes down, as a puff's
/// shadow, body and lit side: stone off masonry and the towers' armour,
/// sand off a sandbag, sawdust off timber, leaves off a tree. `None` for
/// what throws no dust - iron sparks, glass glints, a drum blows up.
pub fn dust_of(material: crate::obstacle::Material) -> Option<[Color; 3]> {
    use crate::obstacle::Material;
    match material {
        Material::Brick | Material::Tesla | Material::GunTower | Material::BioSlush => Some([SMOKE[3], SMOKE[4], SMOKE[5]]),
        Material::Sandbag => Some([DUST[2], DUST[3], DUST[4]]),
        Material::Wood | Material::Fence | Material::Target => Some([DUST[0], DUST[1], rgb(0x99, 0x65, 0x24)]),
        Material::Tree | Material::Pine => Some([rgb(0x1C, 0x4C, 0x33), rgb(0x5F, 0x91, 0x4B), rgb(0x7C, 0x98, 0x3C)]),
        Material::Volcano => Some([SMOKE[0], SMOKE[1], SMOKE[2]]),
        Material::Iron | Material::Glass | Material::Barrel | Material::Lamp | Material::Door => None,
    }
}

/// The one primitive every painter here needs: a filled, alpha-blended
/// box. Every `Canvas` is one; the raylib draw handle is one through
/// `render::pyro::Rl`, so the additive blocks in `Game::render` paint
/// with the same code the CPU tests run.
pub trait Blocks {
    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color);
}

impl<C: Canvas> Blocks for C {
    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        Canvas::fill_rect(self, x, y, width, height, color);
    }
}

/// The 4x4 Bayer threshold of the block at block coordinates (`bx`, `by`),
/// in (0, 1). Anchored to the field, not to whatever is drawn, so two
/// dissolving puffs dither in step and a moving one does not crawl.
pub fn bayer(bx: i32, by: i32) -> f32 {
    const M: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    (M[(by.rem_euclid(4) * 4 + bx.rem_euclid(4)) as usize] as f32 + 0.5) / 16.0
}

/// The block (x, y) in field px falls in, as block coordinates.
pub fn block_of(x: f32, y: f32) -> (i32, i32) {
    ((x / BLOCK).floor() as i32, (y / BLOCK).floor() as i32)
}

/// The even grid line nearest `v`: where a block-built shape is anchored.
pub fn snap(v: f32) -> i32 {
    ((v / BLOCK).round() * BLOCK) as i32
}

/// `c` with its alpha set to `a` (0..1) of full, stepped to eighths, so a
/// translucent layer fades in visible steps rather than smoothly.
pub fn alpha(c: Color, a: f32) -> Color {
    let a = ((a.clamp(0.0, 1.0) * 8.0).round() / 8.0 * c.a as f32) as u8;
    Color::new(c.r, c.g, c.b, a)
}

/// The step of `ramp` at `t` (0 the darkest, 1 the brightest): the nearest
/// step, never a blend of two.
pub fn step(ramp: &[Color], t: f32) -> Color {
    let i = (t.clamp(0.0, 1.0) * (ramp.len() - 1) as f32).round() as usize;
    ramp[i.min(ramp.len() - 1)]
}

/// `step` for the block at (`bx`, `by`), dithered between the two steps
/// `t` falls between - a gradient drawn as bands with pixel-art edges.
pub fn step_dithered(ramp: &[Color], t: f32, bx: i32, by: i32) -> Color {
    let x = t.clamp(0.0, 1.0) * (ramp.len() - 1) as f32;
    let i = x.floor() as usize;
    if i + 1 >= ramp.len() {
        return ramp[ramp.len() - 1];
    }
    let f = x - i as f32;
    if f > bayer(bx, by) { ramp[i + 1] } else { ramp[i] }
}

/// The blocks of a disc of `radius` px around `center`, row by row, as
/// (block row y, first x, last x + BLOCK) spans in field px. The disc is
/// centred on the grid vertex nearest `center`, so it lies on the field's
/// even grid and is symmetric about its centre: every block whose centre
/// is within `radius` is in.
fn disc_rows(center: Position, radius: f32, mut row: impl FnMut(i32, i32, i32, f32)) {
    if radius < BLOCK * 0.75 {
        return;
    }
    let (cx, cy) = (snap(center.x), snap(center.y));
    let m = (radius / BLOCK).ceil() as i32;
    for j in -m..m {
        let dy = (j as f32 + 0.5) * BLOCK;
        let h2 = radius * radius - dy * dy;
        if h2 <= 0.0 {
            continue;
        }
        let h = h2.sqrt();
        let n = (h / BLOCK + 0.5).floor() as i32;
        if n <= 0 {
            continue;
        }
        row(cy + j * B, cx - n * B, cx + n * B, dy);
    }
}

/// A filled disc of whole blocks (see `disc_rows`), one rect per row.
pub fn block_disc(b: &mut impl Blocks, center: Position, radius: f32, color: Color) {
    if color.a == 0 {
        return;
    }
    disc_rows(center, radius, |y, x0, x1, _| b.fill_rect(x0, y, x1 - x0, B, color));
}

/// A disc of blocks only `cover` (0..1) of which are drawn, picked by the
/// field's Bayer pattern: how smoke thins and a puff dissolves. Below half
/// the pattern holds at half and the kept blocks fade in eighths instead,
/// since a sparse dither spread over a wide puff reads as a screen door
/// rather than as smoke thinning. Runs of kept blocks are merged into one
/// rect.
pub fn dither_disc(b: &mut impl Blocks, center: Position, radius: f32, color: Color, cover: f32) {
    if cover >= 0.999 {
        block_disc(b, center, radius, color);
        return;
    }
    if cover <= 0.0 || color.a == 0 {
        return;
    }
    let (cover, color) = if cover < 0.5 { (0.5, alpha(color, cover * 2.0)) } else { (cover, color) };
    if color.a == 0 {
        return;
    }
    disc_rows(center, radius, |y, x0, x1, _| {
        let by = y.div_euclid(B);
        let mut run: Option<i32> = None;
        let mut x = x0;
        while x <= x1 {
            let keep = x < x1 && bayer(x.div_euclid(B), by) < cover;
            match (keep, run) {
                (true, None) => run = Some(x),
                (false, Some(start)) => {
                    b.fill_rect(start, y, x - start, B, color);
                    run = None;
                }
                _ => {}
            }
            x += B;
        }
    });
}

/// A glow in steps: `color` at its full alpha in the middle falling to
/// nothing at `radius`, drawn as `bands` nested discs whose alphas add up
/// (paint it inside an additive blend), each disc's edge dithered across
/// the middle third of its step by the Bayer pattern - the weather light
/// pass's rule (`weather_light.fs`), so a blast's bloom by day and a lamp
/// at night are the same light. `bands` 0 draws nothing.
pub fn glow(b: &mut impl Blocks, center: Position, radius: f32, color: Color, bands: u32) {
    if bands == 0 || color.a == 0 || radius < BLOCK {
        return;
    }
    // A small glow has room for fewer steps: one per 10 px of radius.
    let bands = bands.min(((radius / 10.0) as u32).max(1));
    let k = bands as f32;
    let per = Color::new(color.r, color.g, color.b, ((color.a as f32 / k).round() as u8).max(1));
    let (cx, cy) = (snap(center.x), snap(center.y));
    // Half the width of the dithered zone either side of a band edge: a
    // third of a band, as the light pass dithers.
    let zone = (radius / k) / 6.0;
    for band in 0..bands {
        let r = radius * (1.0 - band as f32 / k);
        if r < BLOCK * 0.75 {
            break;
        }
        let reach = r + zone;
        let m = (reach / BLOCK).ceil() as i32;
        for j in -m..m {
            let dy = (j as f32 + 0.5) * BLOCK;
            let y = cy + j * B;
            let by = y.div_euclid(B);
            // Solid run: every block inside the zone's near edge.
            let inner = r - zone;
            let n_in = if inner * inner > dy * dy { ((inner * inner - dy * dy).sqrt() / BLOCK + 0.5).floor() as i32 } else { 0 };
            if n_in > 0 {
                b.fill_rect(cx - n_in * B, y, 2 * n_in * B, B, per);
            }
            if reach * reach <= dy * dy {
                continue;
            }
            let n_out = ((reach * reach - dy * dy).sqrt() / BLOCK + 0.5).floor() as i32;
            // The dithered blocks between the two: kept where the block's
            // distance is under the edge nudged by its Bayer threshold.
            for i in n_in..n_out {
                for side in [1, -1] {
                    let bx_off = if side > 0 { i } else { -i - 1 };
                    let dx = (bx_off as f32 + 0.5) * BLOCK;
                    let d = (dx * dx + dy * dy).sqrt();
                    let x = cx + bx_off * B;
                    if d <= r + zone * (1.0 - 2.0 * bayer(x.div_euclid(B), by)) {
                        b.fill_rect(x, y, B, B, per);
                    }
                }
            }
        }
    }
}

/// Blocks along the straight line `from` -> `to`, one per block the line
/// crosses, `width` blocks across (1 or more, grown sideways from the
/// line), each coloured by `color_at(t)` with `t` 0 at `from` and 1 at
/// `to`. Consecutive samples in the same block are drawn once.
pub fn block_line(b: &mut impl Blocks, from: Position, to: Position, width: i32, color_at: impl FnMut(f32) -> Color) {
    block_taper(b, from, to, width as f32, width as f32, color_at);
}

/// `block_line` whose width runs from `w_from` blocks at `from` to `w_to`
/// at `to`, rounded to whole blocks and never under one: a tracer, a ray,
/// a flame's tongue.
pub fn block_taper(b: &mut impl Blocks, from: Position, to: Position, w_from: f32, w_to: f32, mut color_at: impl FnMut(f32) -> Color) {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let len = (dx * dx + dy * dy).sqrt();
    let steps = (len / BLOCK).ceil().max(1.0) as i32;
    // Grow a wide line across its own direction: vertically for a line
    // that runs mostly sideways, sideways for one that runs mostly up.
    let across_x = dy.abs() > dx.abs();
    let mut last: Option<(i32, i32, i32)> = None;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let (bx, by) = block_of(from.x + dx * t, from.y + dy * t);
        let width = ((w_from + (w_to - w_from) * t).round() as i32).max(1);
        if last == Some((bx, by, width)) {
            continue;
        }
        last = Some((bx, by, width));
        let c = color_at(t);
        if c.a == 0 {
            continue;
        }
        let lo = -(width - 1) / 2;
        let (x, y, w, h) = if across_x { ((bx + lo) * B, by * B, width * B, B) } else { (bx * B, (by + lo) * B, B, width * B) };
        b.fill_rect(x, y, w, h, c);
    }
}

/// A colour `t` of the way from `a` to `b` (alpha too), in `steps` flat
/// steps rather than a smooth blend: light that fades down a streak.
pub fn between(a: Color, b: Color, t: f32, steps: u32) -> Color {
    let steps = steps.max(1) as f32;
    let k = ((t.clamp(0.0, 1.0) * steps).floor() / steps).min(1.0);
    let l = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * k).round() as u8;
    Color::new(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), l(a.a, b.a))
}

/// One block (`size` 2) or a square of them (`size` 4, 6...) centred on
/// `pos`, snapped to the grid.
pub fn mark(b: &mut impl Blocks, pos: Position, size: i32, color: Color) {
    if color.a == 0 {
        return;
    }
    let half = (size / B).max(1) / 2;
    let (bx, by) = block_of(pos.x, pos.y);
    b.fill_rect((bx - half) * B, (by - half) * B, size.max(B), size.max(B), color);
}

/// Marks along an ellipse of radius `rx` (`rx * squash` tall) around
/// `center`, one block apart, from angle `from` to `to` (radians, +x,
/// clockwise on the y-down field); `stride` 2 or more leaves gaps.
#[allow(clippy::too_many_arguments)]
pub fn ellipse_marks(b: &mut impl Blocks, center: Position, rx: f32, squash: f32, from: f32, to: f32, stride: u32, color: Color) {
    if rx < BLOCK || color.a == 0 {
        return;
    }
    let steps = ((to - from).abs() * rx / BLOCK).ceil().max(1.0) as u32;
    let mut last: Option<(i32, i32)> = None;
    for i in (0..=steps).step_by(stride.max(1) as usize) {
        let a = from + (to - from) * i as f32 / steps as f32;
        let (bx, by) = block_of(center.x + a.cos() * rx, center.y + a.sin() * rx * squash);
        if last == Some((bx, by)) {
            continue;
        }
        last = Some((bx, by));
        b.fill_rect(bx * B, by * B, B, B, color);
    }
}

/// An arc of blocks: every block whose centre lies within `width / 2` px of
/// `radius` round `center` and between the angles `from` and `to` (radians
/// from +x, clockwise on the y-down field, `from <= to`), dissolved like
/// `dither_disc` at `cover` - through the Bayer pattern down to half, then
/// the kept blocks in eighths. Anchored to the field's grid, so an arc that
/// grows does not crawl. A wave's front, a ring of energy.
#[allow(clippy::too_many_arguments)]
pub fn block_arc(b: &mut impl Blocks, center: Position, radius: f32, width: f32, from: f32, to: f32, color: Color, cover: f32) {
    if cover <= 0.0 || color.a == 0 || radius <= 0.0 || to < from {
        return;
    }
    let (cover, color) = if cover < 0.5 { (0.5, alpha(color, cover * 2.0)) } else { (cover.min(1.0), color) };
    if color.a == 0 {
        return;
    }
    let half = (width * 0.5).max(BLOCK * 0.5);
    let outer = radius + half;
    let (bx0, by0) = block_of(center.x - outer, center.y - outer);
    let (bx1, by1) = block_of(center.x + outer, center.y + outer);
    let span = to - from;
    let tau = std::f32::consts::TAU;
    for by in by0..=by1 {
        let mut run: Option<i32> = None;
        for bx in bx0..=bx1 + 1 {
            let keep = bx <= bx1 && {
                let (dx, dy) = ((bx * B) as f32 + BLOCK * 0.5 - center.x, (by * B) as f32 + BLOCK * 0.5 - center.y);
                let d = (dx * dx + dy * dy).sqrt();
                let a = (dy.atan2(dx) - from).rem_euclid(tau);
                (d - radius).abs() <= half && (a <= span || span >= tau) && (cover >= 0.999 || bayer(bx, by) < cover)
            };
            match (keep, run) {
                (true, None) => run = Some(bx),
                (false, Some(start)) => {
                    b.fill_rect(start * B, by * B, (bx - start) * B, B, color);
                    run = None;
                }
                _ => {}
            }
        }
    }
}

/// One shaded puff: `body`, then `shadow` - a disc down and right in a
/// darker step, under the body - then `lit`, a smaller disc up and left in
/// a lighter step, then `core`, fire still burning inside, as a colour and
/// a fraction of the radius, a little below centre. `cover` below 1 draws
/// the whole puff dissolved through the Bayer pattern, which is how smoke
/// thins away. Shaded rather than outlined: an outline reads as a sticker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Puff {
    pub pos: Position,
    pub radius: f32,
    pub body: Color,
    pub shadow: Option<Color>,
    pub lit: Option<Color>,
    pub core: Option<(Color, f32)>,
    pub cover: f32,
}

impl Puff {
    /// A solid puff of one colour.
    pub fn plain(pos: Position, radius: f32, body: Color) -> Self {
        Puff { pos, radius, body, shadow: None, lit: None, core: None, cover: 1.0 }
    }
}

/// What a composer hands its painter, in painting order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Puff(Puff),
    /// A single block (`size` 2) or a square of them: ring points,
    /// sparks, debris, the heads of streaks.
    Mark { pos: Position, size: i32, color: Color },
    /// Light: a stepped glow (`glow`), drawn in the additive pass.
    Glow { pos: Position, radius: f32, color: Color },
    /// A streak of blocks from `from` to `to`, `width` blocks across at
    /// `from` narrowing to one, stepping from `head` to `tail`: rays,
    /// debris, the tongues of a flame.
    Line { from: Position, to: Position, width: f32, head: Color, tail: Color },
    /// An arc of blocks (`block_arc`): a wave's front, a ring.
    Arc { center: Position, radius: f32, width: f32, from: f32, to: f32, color: Color, cover: f32 },
}

/// Paint `shapes` in order: puffs and marks alpha-blended. Glows are left
/// out; `draw_glows` paints them inside an additive blend.
pub fn draw(b: &mut impl Blocks, shapes: &[Shape]) {
    for shape in shapes {
        match *shape {
            Shape::Puff(p) => draw_puff(b, &p),
            Shape::Mark { pos, size, color } => mark(b, pos, size, color),
            Shape::Line { from, to, width, head, tail } => block_taper(b, from, to, width, 1.0, |t| between(head, tail, t, 3)),
            Shape::Arc { center, radius, width, from, to, color, cover } => block_arc(b, center, radius, width, from, to, color, cover),
            Shape::Glow { .. } => {}
        }
    }
}

/// The glows among `shapes`, in `bands` steps (inside an additive blend).
pub fn draw_glows(b: &mut impl Blocks, shapes: &[Shape], bands: u32) {
    for shape in shapes {
        if let Shape::Glow { pos, radius, color } = *shape {
            glow(b, pos, radius, color, bands);
        }
    }
}

/// One puff: shadow, body, lit side, core (see `Puff`).
pub fn draw_puff(b: &mut impl Blocks, p: &Puff) {
    let r = p.radius;
    if r < BLOCK * 0.75 || p.cover <= 0.0 {
        return;
    }
    if let Some(shadow) = p.shadow {
        dither_disc(b, Position::new(p.pos.x + r * 0.16, p.pos.y + r * 0.2), r * 0.94, shadow, p.cover);
    }
    dither_disc(b, p.pos, r, p.body, p.cover);
    if let Some(lit) = p.lit {
        dither_disc(b, Position::new(p.pos.x - r * 0.2, p.pos.y - r * 0.26), r * 0.62, lit, p.cover);
    }
    if let Some((core, frac)) = p.core {
        dither_disc(b, Position::new(p.pos.x, p.pos.y + r * 0.12), r * frac, core, p.cover);
    }
}

/// The shading of a puff by `heat` (1 white hot, 0 burnt out) and `ash`
/// (0 soot, 1 pale): fire above `FIREBALL`, smoke with a dying core below
/// it. `sunlit` gives smoke its lit side (the field is lit from up and
/// left, as the shadows fall), and every puff gets a shadow step.
pub fn shade(heat: f32, ash: f32, sunlit: bool) -> (Color, Option<Color>, Option<Color>, Option<(Color, f32)>) {
    if heat > FIREBALL {
        // Fire: the body steps from gold through red as it cools, the core
        // from white to gold; the shadow is the next step down.
        let q = ((heat - FIREBALL) / (1.0 - FIREBALL)).clamp(0.0, 1.0);
        let i = 3 + (q * 3.0).round() as usize; // 3..=6
        let body = FIRE[i];
        let shadow = FIRE[i - 2];
        let core = FIRE[(i + 2).min(7)];
        return (body, Some(shadow), None, Some((core, 0.42 + 0.3 * q)));
    }
    let s = 1 + ((ash.clamp(0.0, 1.0) * 3.99) as usize).min(3); // 1..=4
    let body = SMOKE[s];
    let shadow = SMOKE[s - 1];
    let lit = sunlit.then_some(SMOKE[s + 1]);
    let core = (heat > 0.0).then(|| (if heat > 0.3 { FIRE[3] } else { FIRE[2] }, 0.2 + 0.5 * heat / FIREBALL));
    (body, Some(shadow), lit, core)
}

/// Heat above this and a puff is still part of the fireball; below it the
/// puff is smoke with a fire core dying inside it.
pub const FIREBALL: f32 = 0.55;

/// A shaded puff of fire or smoke at `pos` (see `shade`).
pub fn fire_puff(pos: Position, radius: f32, heat: f32, ash: f32, sunlit: bool, cover: f32) -> Puff {
    let (body, shadow, lit, core) = shade(heat, ash, sunlit);
    Puff { pos, radius, body, shadow, lit, core, cover }
}

/// A shaded puff of dust (`t` 0 dark earth, 1 pale sand), `cover` of
/// its blocks kept - dust is thin.
pub fn dust_puff(pos: Position, radius: f32, t: f32, cover: f32) -> Puff {
    let i = 1 + ((t.clamp(0.0, 1.0) * 1.99) as usize); // 1..=2
    Puff { pos, radius, body: DUST[i], shadow: Some(DUST[i - 1]), lit: Some(DUST[i + 1]), core: None, cover }
}

/// How far smoke drifts sideways for every px it rises at `pos` at
/// `time`: the wind that bends the grass (`grass::wind_at`) as a lean, at
/// `smoke_wind_drift` in a typical gust - so a smoke column leans and
/// swings with the tufts under it. Signed: positive drifts east.
pub fn smoke_lean(t: &crate::tuning::Tuning, pos: Position, time: f32) -> f32 {
    t.smoke_wind_drift * crate::grass::wind_at(t, pos, time) / WIND_TYPICAL_PX
}

/// The grass's lean (px) in a typical gust, which `smoke_wind_drift` is
/// measured against.
const WIND_TYPICAL_PX: f32 = 3.5;

/// Flames burning from the ground at `base`: `count` tongues spread across
/// `spread` px, each up to `height` px tall, leaning `lean` px sideways
/// per px of height with the wind (`smoke_lean`), `strength` (0..1)
/// shrinking them as a fire catches or dies down. A tongue is a teardrop -
/// a round belly at the root drawn up into a point - in three nested
/// layers: a red outer flame, a gold body, a pale core low in the belly.
/// Each hashes its place and size from `seed` and flickers through four
/// heights, one step every four frames of `time`, out of step with its
/// neighbours. Appended to `out`, back to front.
#[allow(clippy::too_many_arguments)]
pub fn tongues(out: &mut Vec<Shape>, base: Position, spread: f32, height: f32, count: u32, seed: u32, time: f32, lean: f32, strength: f32) {
    const FLICKER: [f32; 4] = [1.0, 0.82, 1.1, 0.9];
    let strength = strength.clamp(0.0, 1.0);
    if strength <= 0.0 || height < BLOCK {
        return;
    }
    let tick = (time * 60.0 / 4.0).floor() as i64;
    for i in 0..count {
        let s = 900 + i * 11;
        let x = base.x + (unit(seed, s) - 0.5) * spread;
        let phase = (unit(seed, s + 1) * 4.0) as i64;
        let flick = FLICKER[((tick + phase + i as i64).rem_euclid(4)) as usize];
        let size = (0.6 + 0.4 * unit(seed, s + 2)) * (0.4 + 0.6 * strength);
        let h = height * size * flick;
        let belly = (height * 0.2 * size).max(BLOCK);
        if h < BLOCK * 2.0 {
            continue;
        }
        // The tip sways a block either way with the flicker, over the lean.
        let sway = match (tick + phase).rem_euclid(4) {
            0 => BLOCK,
            2 => -BLOCK,
            _ => 0.0,
        };
        let root = Position::new(x, base.y - belly * 0.8);
        let tip = |k: f32| Position::new(x + (lean * h + sway) * k, base.y - h * k);
        let across = belly * 2.0 / BLOCK;
        for (k, grow, color, tip_color) in [(1.0, 1.0, FIRE[3], FIRE[2]), (0.72, 0.68, FIRE[5], FIRE[4]), (0.42, 0.4, FIRE[7], FIRE[6])] {
            let r = belly * grow;
            if r < BLOCK * 0.75 {
                continue;
            }
            out.push(Shape::Puff(Puff::plain(Position::new(root.x, root.y + belly * (1.0 - grow) * 0.5), r, color)));
            out.push(Shape::Line { from: root, to: tip(k), width: (across * grow).max(1.0), head: color, tail: tip_color });
        }
    }
}

/// 0..1 from a seed and a salt: a cosmetic choice hashed rather than
/// rolled, so replays, replicas and tests agree (`blast::hash_unit`).
pub fn unit(seed: u32, k: u32) -> f32 {
    crate::blast::hash_unit(seed, k)
}

/// Cubic ease-out: fast, then settling.
pub fn ease_out(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    1.0 - (1.0 - x) * (1.0 - x) * (1.0 - x)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canvas that records every rect it is asked to fill.
    #[derive(Default)]
    struct Rects(Vec<(i32, i32, i32, i32, Color)>);

    impl Blocks for Rects {
        fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
            self.0.push((x, y, w, h, c));
        }
    }

    fn on_grid(r: &Rects) -> bool {
        r.0.iter().all(|&(x, y, w, h, _)| x % 2 == 0 && y % 2 == 0 && w % 2 == 0 && h % 2 == 0 && w > 0 && h > 0)
    }

    #[test]
    fn every_ramp_runs_dark_to_bright() {
        let lum = |c: Color| 0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32;
        for ramp in [&FIRE[..], &SMOKE[..], &DUST[..], &PLASMA_TEAL[..], &PLASMA_PURPLE[..], &TESLA[..], &LASER_RED[..], &LASER_BLUE[..], &OOZE[..]] {
            for w in ramp.windows(2) {
                assert!(lum(w[1]) > lum(w[0]), "{:?} then {:?}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn a_disc_lies_on_the_even_grid_and_is_symmetric() {
        for r in [2.0, 3.0, 5.5, 9.0, 17.3] {
            for (x, y) in [(10.0, 10.0), (11.3, 7.9), (100.9, 51.1)] {
                let mut rects = Rects::default();
                block_disc(&mut rects, Position::new(x, y), r, FIRE[3]);
                assert!(on_grid(&rects), "r {r} at ({x}, {y})");
                let (cx, cy) = (snap(x), snap(y));
                for &(x0, y0, w, _, _) in &rects.0 {
                    assert_eq!(cx - x0, x0 + w - cx, "each row is centred");
                    assert!(rects.0.iter().any(|&(_, y1, w1, _, _)| y1 == 2 * cy - y0 - 2 && w1 == w), "rows mirror top to bottom");
                }
            }
        }
    }

    #[test]
    fn a_dissolve_keeps_about_its_share_of_the_light() {
        // Blocks kept, each weighted by how opaque it is drawn.
        let light = |cover: f32| {
            let mut r = Rects::default();
            dither_disc(&mut r, Position::new(40.0, 40.0), 20.0, SMOKE[3], cover);
            assert!(on_grid(&r));
            r.0.iter().map(|&(_, _, w, _, c)| (w / 2) as f32 * c.a as f32 / 255.0).sum::<f32>()
        };
        let full = light(1.0);
        for cover in [0.25, 0.5, 0.75] {
            let share = light(cover) / full;
            assert!((share - cover).abs() < 0.08, "cover {cover} kept {share}");
        }
        assert_eq!(light(0.0), 0.0);
        // Below half the pattern holds and the blocks fade: never a sparse
        // screen of single blocks.
        let mut r = Rects::default();
        dither_disc(&mut r, Position::new(40.0, 40.0), 20.0, SMOKE[3], 0.2);
        assert!(r.0.iter().all(|&(.., c)| c.a < 255));
    }

    #[test]
    fn a_glow_is_brightest_in_the_middle_in_whole_steps() {
        let mut r = Rects::default();
        glow(&mut r, Position::new(64.0, 64.0), 40.0, Color::new(255, 150, 60, 200), 4);
        assert!(on_grid(&r));
        // Add the alphas the way an additive blend would, per block.
        let mut at = std::collections::HashMap::new();
        for &(x, y, w, h, c) in &r.0 {
            for bx in (x..x + w).step_by(2) {
                for by in (y..y + h).step_by(2) {
                    *at.entry((bx, by)).or_insert(0u32) += c.a as u32;
                }
            }
        }
        let centre = at[&(64, 64)];
        assert_eq!(centre, 200, "the bands add up to the colour's alpha in the middle");
        let levels: std::collections::BTreeSet<u32> = at.values().copied().collect();
        assert!(levels.len() <= 4, "four bands, four levels: {levels:?}");
        assert!(!at.contains_key(&(64 + 44, 64)), "nothing past the radius");
    }

    #[test]
    fn a_line_is_one_block_per_step_and_colours_head_to_tail() {
        let mut r = Rects::default();
        block_line(&mut r, Position::new(0.0, 0.0), Position::new(20.0, 0.0), 1, |t| if t < 0.5 { FIRE[6] } else { FIRE[2] });
        assert!(on_grid(&r));
        assert_eq!(r.0.len(), 11, "0..=20 px is eleven blocks");
        assert_eq!(r.0[0].4, FIRE[6]);
        assert_eq!(r.0[10].4, FIRE[2]);
    }

    #[test]
    fn steps_never_blend() {
        for t in [0.0, 0.1, 0.33, 0.5, 0.77, 1.0] {
            assert!(FIRE.contains(&step(&FIRE, t)));
            for (bx, by) in [(0, 0), (1, 3), (7, 2)] {
                assert!(SMOKE.contains(&step_dithered(&SMOKE, t, bx, by)));
            }
        }
    }

    #[test]
    fn fire_cools_into_smoke() {
        let (hot, ..) = shade(1.0, 0.0, false);
        let (warm, ..) = shade(0.6, 0.0, false);
        let (smoke, _, lit, core) = shade(0.2, 0.5, true);
        assert!(FIRE.contains(&hot) && FIRE.contains(&warm));
        assert!(SMOKE.contains(&smoke) && lit.is_some() && core.is_some());
        let (ash, _, _, none) = shade(0.0, 1.0, false);
        assert!(SMOKE.contains(&ash) && none.is_none());
    }
}
