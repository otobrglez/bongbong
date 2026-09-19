//! The drawing surface the static field is painted on (docs/mapshot-prd.md).
//!
//! The leaf draw functions of the field - ground, grass, obstacles, frogs,
//! pickups, tanks and their rings, tread marks, scorches, rubble - are
//! generic over [`Canvas`] rather than raylib's `RaylibDraw`, so the same
//! code paints two very different targets:
//!
//! - `render::canvas::GpuCanvas` wraps a raylib draw handle and the real
//!   `Texture2D`s (behind the `render` feature). Every method is one raylib
//!   call, so `Game::render` draws exactly what the trait describes.
//! - [`CpuCanvas`] is a nearest-neighbour rasteriser over a plain pixel
//!   buffer of `math::Color`, fed by sprite sheets decoded into [`Pixels`]
//!   (no window, no GL context). It is what `mapshot` uses on a machine
//!   with no display, and what the map thumbnail tests run under
//!   `cargo test --lib`. The rasteriser itself needs no raylib; decoding a
//!   PNG into `Pixels` and encoding the canvas back out are raylib's image
//!   functions, so those live in `render::canvas` too.
//!
//! Sheets are named by [`Sheet`], not passed as texture refs, so a draw
//! function says *what* it blits and the canvas owns *where from*. The one
//! path table, [`Sheet::path`], is what every loader reads.
//!
//! Why not raylib's own `ImageDraw` for the CPU side: it resizes with a
//! linear filter when source and destination sizes differ (tanks draw at
//! 2x, grass and pickups at their own scales - every scaled sprite would
//! blur), it ignores a negative source width (no mirroring, which grass and
//! frogs rely on), `ImageDrawRectangle` copies bytes without blending, and
//! `ImageDrawText` lazily loads the default font through a GL texture. So
//! raylib only decodes PNGs and encodes the result.
//!
//! There is deliberately no text method: the only text on the field is the
//! transient `P1`/`P2` locate label, which stays raylib-only
//! (`render::tank::draw_player_label`).

use crate::frog::{FROG_VARIANT_DIRS, FrogAnim};
use crate::map::Theme;
use crate::pickup::PickupKind;
use crate::Position;
use crate::math::{Color, Rectangle, Vec2};
use std::collections::BTreeMap;

/// A sprite sheet the field draws from. The canvas owns the backing store:
/// a `Texture2D` on the GPU, decoded pixels on the CPU.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Sheet {
    /// The ground tileset of a theme (`Theme::ground_texture_path`,
    /// ground.rs): one file per theme, so a loader that wants every sheet
    /// gets both and the field names the map's.
    Ground(Theme),
    /// static/scifi_tanks_sheet.png (docs/SPRITESHEET_SPEC.md).
    Tanks,
    /// static/walls_sheet.png (docs/WALLS_SPEC.md); rubble rows too.
    Walls,
    /// static/props_sheet.png (docs/PROPS_SPEC.md); oil cells too.
    Props,
    /// static/trees_sheet.png (docs/TREES_SPEC.md).
    Trees,
    /// The tall-grass sheet of a theme (`Theme::grass_texture_path`,
    /// grass.rs), one file per theme like `Ground`.
    Grass(Theme),
    /// static/damage.png - the hull damage overlays (damage_stage.rs).
    Damage,
    /// static/minigun_mount.png - the barrel cluster on a turret.
    MinigunMount,
    /// static/tracks.png - one tread mark.
    Tracks,
    /// static/barrel_explosion.png - the blast frames and scorches (blast.rs).
    BarrelExplosion,
    /// static/portal_sheet.png - the turning spiral (portal.rs).
    Portal,
    /// static/pickups/<kind>.png - each kind its own 32 x 32 image.
    Pickup(PickupKind),
    /// static/toxic_frog/<variant dir>/<clip>.png - one filmstrip per clip
    /// per colour variant (`FROG_VARIANT_DIRS`, docs/FROG_SPEC.md).
    Frog { variant: u8, clip: FrogAnim },
}

/// The nine sheets that are one file each regardless of theme.
pub const SINGLE_SHEETS: [Sheet; 9] = [
    Sheet::Tanks,
    Sheet::Walls,
    Sheet::Props,
    Sheet::Trees,
    Sheet::Damage,
    Sheet::MinigunMount,
    Sheet::Tracks,
    Sheet::BarrelExplosion,
    Sheet::Portal,
];

/// Every pickup kind, each its own sheet (`pickup_file` is exhaustive over
/// the enum, so a new kind without a row here fails to compile there).
pub const PICKUP_KINDS: [PickupKind; 9] = [
    PickupKind::Health,
    PickupKind::Ammo,
    PickupKind::Laser,
    PickupKind::Minigun,
    PickupKind::Plasma,
    PickupKind::SpeedUp,
    PickupKind::Shield,
    PickupKind::Flamethrower,
    PickupKind::FrogHealth,
];

/// The five frog clips, in `FrogAnim` order.
pub const FROG_CLIPS: [FrogAnim; 5] = [FrogAnim::Idle, FrogAnim::Hurt, FrogAnim::Hop, FrogAnim::Attack, FrogAnim::Explosion];

impl Sheet {
    /// The asset's path relative to the working directory - the one table
    /// every loader (the game, the demos, the CPU canvas) reads.
    pub fn path(self) -> String {
        match self {
            Sheet::Ground(theme) => theme.ground_texture_path().into(),
            Sheet::Tanks => "static/scifi_tanks_sheet.png".into(),
            Sheet::Walls => "static/walls_sheet.png".into(),
            Sheet::Props => "static/props_sheet.png".into(),
            Sheet::Trees => "static/trees_sheet.png".into(),
            Sheet::Grass(theme) => theme.grass_texture_path().into(),
            Sheet::Damage => "static/damage.png".into(),
            Sheet::MinigunMount => "static/minigun_mount.png".into(),
            Sheet::Tracks => "static/tracks.png".into(),
            Sheet::BarrelExplosion => "static/barrel_explosion.png".into(),
            Sheet::Portal => "static/portal_sheet.png".into(),
            Sheet::Pickup(kind) => format!("static/pickups/{}.png", pickup_file(kind)),
            Sheet::Frog { variant, clip } => {
                let dir = FROG_VARIANT_DIRS[variant as usize % FROG_VARIANT_DIRS.len()];
                format!("static/toxic_frog/{dir}/{}.png", frog_clip_file(clip))
            }
        }
    }

    /// Every sheet the field can ask for: the ground and grass sheets of
    /// every theme, the single sheets, one per pickup kind, and every
    /// frog variant's five clips.
    pub fn all() -> Vec<Sheet> {
        let mut all: Vec<Sheet> = Theme::ALL.iter().flat_map(|&t| [Sheet::Ground(t), Sheet::Grass(t)]).collect();
        all.extend(SINGLE_SHEETS);
        all.extend(PICKUP_KINDS.iter().map(|&k| Sheet::Pickup(k)));
        for variant in 0..FROG_VARIANT_DIRS.len() as u8 {
            all.extend(FROG_CLIPS.iter().map(|&clip| Sheet::Frog { variant, clip }));
        }
        all
    }
}

/// The pickup icon file stem for a kind (static/pickups/SOURCE.md).
fn pickup_file(kind: PickupKind) -> &'static str {
    match kind {
        PickupKind::Health => "health",
        PickupKind::Ammo => "ammo",
        PickupKind::Laser => "laser",
        PickupKind::Minigun => "minigun",
        PickupKind::Plasma => "plasma",
        PickupKind::SpeedUp => "speedup",
        PickupKind::Shield => "shield",
        PickupKind::Flamethrower => "flamethrower",
        PickupKind::FrogHealth => "frog_health",
    }
}

/// The filmstrip file stem for a frog clip.
fn frog_clip_file(clip: FrogAnim) -> &'static str {
    match clip {
        FrogAnim::Idle => "idle",
        FrogAnim::Hurt => "hurt",
        FrogAnim::Hop => "hop",
        FrogAnim::Attack => "attack",
        FrogAnim::Explosion => "explosion",
    }
}

/// What a field draw function can do. Semantics follow the raylib calls
/// the GPU canvas forwards to, so the CPU canvas has one target to match.
pub trait Canvas {
    /// `DrawTexturePro`: `dest.x/y` is where `origin` (in destination
    /// pixels) lands, `rotation` is degrees clockwise on the y-down screen,
    /// a negative `src.width`/`src.height` mirrors along that axis, `tint`
    /// multiplies the texel, and the result is alpha-blended over.
    fn blit(&mut self, sheet: Sheet, src: Rectangle, dest: Rectangle, origin: Vec2, rotation: f32, tint: Color);
    /// `DrawRectangle`: a filled, alpha-blended box.
    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color);
    /// `DrawRectangleGradientV`: `top` at the first row to `bottom` at the last.
    fn gradient_v(&mut self, x: i32, y: i32, width: i32, height: i32, top: Color, bottom: Color);
    /// `DrawRectangleGradientH`: `left` at the first column to `right` at the last.
    fn gradient_h(&mut self, x: i32, y: i32, width: i32, height: i32, left: Color, right: Color);
    /// `DrawCircleV`: a filled disc.
    fn disc(&mut self, center: Position, radius: f32, color: Color);
    /// `DrawRing`: an annulus between `inner` and `outer`, from `start_deg`
    /// to `end_deg` measured from +x, clockwise on the y-down screen.
    /// `segments` is the polygon density the GPU draws with; the CPU draws
    /// the true annulus and ignores it.
    #[allow(clippy::too_many_arguments)]
    fn ring(&mut self, center: Position, inner: f32, outer: f32, start_deg: f32, end_deg: f32, segments: i32, color: Color);
}

/// A decoded sprite sheet: RGBA pixels, row-major.
#[derive(Clone, Debug)]
pub struct Pixels {
    pub width: usize,
    pub height: usize,
    pub data: Vec<Color>,
}

impl Pixels {
    /// A blank sheet, for tests and for `CpuCanvas::blank`.
    pub fn filled(width: usize, height: usize, color: Color) -> Self {
        Pixels { width, height, data: vec![color; width * height] }
    }

    #[inline]
    fn at(&self, x: usize, y: usize) -> Color {
        self.data[y * self.width + x]
    }
}

/// A [`Canvas`] over a plain pixel buffer: a nearest-neighbour rasteriser
/// with alpha-over blending that matches what raylib's point-filtered
/// textures and `BLEND_ALPHA` produce, to within rounding. See the module
/// doc for why it exists.
pub struct CpuCanvas {
    width: usize,
    height: usize,
    pixels: Vec<Color>,
    sheets: BTreeMap<Sheet, Pixels>,
}

impl CpuCanvas {
    /// A white canvas with the given sheets. `Sheet`s the map never asks
    /// for can be left out; a blit from a missing sheet draws nothing.
    pub fn new(width: usize, height: usize, sheets: BTreeMap<Sheet, Pixels>) -> Self {
        CpuCanvas { width, height, pixels: vec![Color::WHITE; width * height], sheets }
    }

    /// A white canvas with no sheets at all - rectangles, discs and rings
    /// still draw. For the rasteriser's own tests.
    pub fn blank(width: usize, height: usize) -> Self {
        Self::new(width, height, BTreeMap::new())
    }

    pub fn insert_sheet(&mut self, sheet: Sheet, pixels: Pixels) {
        self.sheets.insert(sheet, pixels);
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn pixels(&self) -> &[Color] {
        &self.pixels
    }

    pub fn pixel(&self, x: usize, y: usize) -> Color {
        self.pixels[y * self.width + x]
    }

    pub fn clear(&mut self, color: Color) {
        self.pixels.fill(color);
    }

    /// A deterministic FNV-1a over the RGBA bytes, for pinning a render in
    /// a test.
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for c in &self.pixels {
            for b in [c.r, c.g, c.b, c.a] {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    #[inline]
    fn blend(&mut self, x: usize, y: usize, src: Color) {
        blend_into(&mut self.pixels, self.width, x, y, src);
    }

    /// The canvas rows/columns a box touches, clipped: `[x0, x1)`.
    fn clip(&self, x: i32, y: i32, width: i32, height: i32) -> Option<(usize, usize, usize, usize)> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + width).min(self.width as i32);
        let y1 = (y + height).min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some((x0 as usize, y0 as usize, x1 as usize, y1 as usize))
    }
}

/// Alpha-over `src` onto the pixel at (x, y) of a `width`-wide buffer,
/// raylib `BLEND_ALPHA`: `out = src * a + dst * (1 - a)` per channel, alpha
/// the same way. A free function so `blit` can hold a sheet borrowed from
/// the canvas while it writes the canvas's pixels.
#[inline]
fn blend_into(pixels: &mut [Color], width: usize, x: usize, y: usize, src: Color) {
    if src.a == 0 {
        return;
    }
    let i = y * width + x;
    let dst = pixels[i];
    if src.a == 255 {
        pixels[i] = src;
        return;
    }
    let a = src.a as f32 / 255.0;
    let mix = |s: u8, d: u8| (s as f32 * a + d as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    let out_a = (src.a as f32 + dst.a as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    pixels[i] = Color::new(mix(src.r, dst.r), mix(src.g, dst.g), mix(src.b, dst.b), out_a);
}

/// `texel * tint`, channel by channel, the way the GPU multiplies a
/// fragment by its vertex colour.
#[inline]
fn tinted(texel: Color, tint: Color) -> Color {
    if tint.r == 255 && tint.g == 255 && tint.b == 255 && tint.a == 255 {
        return texel;
    }
    let mul = |a: u8, b: u8| ((a as u32 * b as u32 + 127) / 255) as u8;
    Color::new(mul(texel.r, tint.r), mul(texel.g, tint.g), mul(texel.b, tint.b), mul(texel.a, tint.a))
}

/// Linear interpolation between two colours at `t` in `0..=1`.
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color::new(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b), l(a.a, b.a))
}

impl Canvas for CpuCanvas {
    fn blit(&mut self, sheet: Sheet, src: Rectangle, dest: Rectangle, origin: Vec2, rotation: f32, tint: Color) {
        let Some(px) = self.sheets.get(&sheet) else { return };
        let (flip_x, flip_y) = (src.width < 0.0, src.height < 0.0);
        let (sw, sh) = (src.width.abs(), src.height.abs());
        if sw <= 0.0 || sh <= 0.0 || dest.width <= 0.0 || dest.height <= 0.0 || px.width == 0 || px.height == 0 {
            return;
        }
        // The quad's corners in canvas space: `dest.x/y` plus the origin-
        // relative corner rotated by `rotation` (raylib's own construction).
        // `trig::sin_cos`, not libm's: an ulp of platform difference here
        // moves a nearest-neighbour sample across a pixel boundary, and
        // the pinned thumbnail hashes have to agree between a Mac and the
        // Linux CI runner.
        let (sin, cos) = crate::trig::sin_cos(rotation.to_radians());
        let corners = [(-origin.x, -origin.y), (dest.width - origin.x, -origin.y), (-origin.x, dest.height - origin.y), (dest.width - origin.x, dest.height - origin.y)];
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (dx, dy) in corners {
            let wx = dest.x + dx * cos - dy * sin;
            let wy = dest.y + dx * sin + dy * cos;
            min_x = min_x.min(wx);
            min_y = min_y.min(wy);
            max_x = max_x.max(wx);
            max_y = max_y.max(wy);
        }
        let x0 = (min_x.floor() as i32).max(0);
        let y0 = (min_y.floor() as i32).max(0);
        let x1 = (max_x.ceil() as i32).min(self.width as i32);
        let y1 = (max_y.ceil() as i32).min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let sheet_w = px.width as i32;
        let sheet_h = px.height as i32;
        let width = self.width;
        for py in y0..y1 {
            for pxx in x0..x1 {
                // Inverse-map the pixel centre into the unrotated quad.
                let vx = pxx as f32 + 0.5 - dest.x;
                let vy = py as f32 + 0.5 - dest.y;
                let lx = vx * cos + vy * sin + origin.x;
                let ly = -vx * sin + vy * cos + origin.y;
                let u = lx / dest.width;
                let v = ly / dest.height;
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                let su = if flip_x { 1.0 - u } else { u };
                let sv = if flip_y { 1.0 - v } else { v };
                let sx = (src.x + su * sw).floor() as i32;
                let sy = (src.y + sv * sh).floor() as i32;
                if sx < 0 || sy < 0 || sx >= sheet_w || sy >= sheet_h {
                    continue;
                }
                let texel = px.at(sx as usize, sy as usize);
                blend_into(&mut self.pixels, width, pxx as usize, py as usize, tinted(texel, tint));
            }
        }
    }

    fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: Color) {
        let Some((x0, y0, x1, y1)) = self.clip(x, y, width, height) else { return };
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, color);
            }
        }
    }

    fn gradient_v(&mut self, x: i32, y: i32, width: i32, height: i32, top: Color, bottom: Color) {
        let Some((x0, y0, x1, y1)) = self.clip(x, y, width, height) else { return };
        for py in y0..y1 {
            let t = (py as f32 + 0.5 - y as f32) / height as f32;
            let color = lerp_color(top, bottom, t);
            for px in x0..x1 {
                self.blend(px, py, color);
            }
        }
    }

    fn gradient_h(&mut self, x: i32, y: i32, width: i32, height: i32, left: Color, right: Color) {
        let Some((x0, y0, x1, y1)) = self.clip(x, y, width, height) else { return };
        for px in x0..x1 {
            let t = (px as f32 + 0.5 - x as f32) / width as f32;
            let color = lerp_color(left, right, t);
            for py in y0..y1 {
                self.blend(px, py, color);
            }
        }
    }

    fn disc(&mut self, center: Position, radius: f32, color: Color) {
        if radius <= 0.0 {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.clip(
            (center.x - radius).floor() as i32,
            (center.y - radius).floor() as i32,
            (radius * 2.0).ceil() as i32 + 1,
            (radius * 2.0).ceil() as i32 + 1,
        ) else {
            return;
        };
        let r2 = radius * radius;
        for py in y0..y1 {
            for px in x0..x1 {
                let dx = px as f32 + 0.5 - center.x;
                let dy = py as f32 + 0.5 - center.y;
                if dx * dx + dy * dy <= r2 {
                    self.blend(px, py, color);
                }
            }
        }
    }

    fn ring(&mut self, center: Position, inner: f32, outer: f32, start_deg: f32, end_deg: f32, _segments: i32, color: Color) {
        // raylib's own normalisation: swapped radii and angles are fine, a
        // zero sweep draws nothing.
        let (inner, outer) = if inner > outer { (outer, inner) } else { (inner, outer) };
        let (start, end) = if start_deg > end_deg { (end_deg, start_deg) } else { (start_deg, end_deg) };
        let sweep = end - start;
        if outer <= 0.0 || sweep <= 0.0 {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.clip(
            (center.x - outer).floor() as i32,
            (center.y - outer).floor() as i32,
            (outer * 2.0).ceil() as i32 + 1,
            (outer * 2.0).ceil() as i32 + 1,
        ) else {
            return;
        };
        let (r_in2, r_out2) = (inner * inner, outer * outer);
        let full = sweep >= 360.0;
        for py in y0..y1 {
            for px in x0..x1 {
                let dx = px as f32 + 0.5 - center.x;
                let dy = py as f32 + 0.5 - center.y;
                let d2 = dx * dx + dy * dy;
                if d2 < r_in2 || d2 > r_out2 {
                    continue;
                }
                if !full {
                    let angle = crate::trig::atan2(dy, dx).to_degrees();
                    let rel = (angle - start).rem_euclid(360.0);
                    if rel > sweep {
                        continue;
                    }
                }
                self.blend(px, py, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = Color::new(255, 0, 0, 255);
    const GREEN: Color = Color::new(0, 255, 0, 255);
    const BLUE: Color = Color::new(0, 0, 255, 255);
    const CLEAR: Color = Color::new(0, 0, 0, 0);
    const WHITE: Color = Color::WHITE;

    /// `Color` derives no `PartialEq`, so the tests compare tuples.
    fn rgba(c: Color) -> (u8, u8, u8, u8) {
        (c.r, c.g, c.b, c.a)
    }

    trait Px {
        fn px(&self, x: usize, y: usize) -> (u8, u8, u8, u8);
    }

    impl Px for CpuCanvas {
        fn px(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
            rgba(self.pixel(x, y))
        }
    }

    /// A 4 x 4 sprite: red top-left texel, green top-right, blue bottom-left,
    /// everything else transparent - asymmetric on both axes.
    fn probe_sheet() -> Pixels {
        let mut p = Pixels::filled(4, 4, CLEAR);
        p.data[0] = RED;
        p.data[3] = GREEN;
        p.data[12] = BLUE;
        p
    }

    fn canvas_with_probe() -> CpuCanvas {
        let mut c = CpuCanvas::blank(16, 16);
        c.insert_sheet(Sheet::Tracks, probe_sheet());
        c
    }

    #[test]
    fn plain_blit_lands_texels_where_the_gpu_would() {
        let mut c = canvas_with_probe();
        c.blit(Sheet::Tracks, Rectangle::new(0.0, 0.0, 4.0, 4.0), Rectangle::new(4.0, 4.0, 4.0, 4.0), Vec2::zero(), 0.0, Color::WHITE);
        assert_eq!(c.px(4, 4), rgba(RED));
        assert_eq!(c.px(7, 4), rgba(GREEN));
        assert_eq!(c.px(4, 7), rgba(BLUE));
        assert_eq!(c.px(7, 7), rgba(Color::WHITE), "transparent texel leaves the background");
        assert_eq!(c.px(3, 4), rgba(Color::WHITE), "nothing outside the destination");
    }

    #[test]
    fn scaled_blit_is_nearest_neighbour() {
        let mut c = canvas_with_probe();
        c.blit(Sheet::Tracks, Rectangle::new(0.0, 0.0, 4.0, 4.0), Rectangle::new(0.0, 0.0, 8.0, 8.0), Vec2::zero(), 0.0, Color::WHITE);
        // Every texel becomes a crisp 2 x 2 block, no blending between them.
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            assert_eq!(c.px(x, y), rgba(RED));
        }
        assert_eq!(c.px(6, 0), rgba(GREEN));
        assert_eq!(c.px(7, 1), rgba(GREEN));
        assert_eq!(c.px(2, 0), rgba(Color::WHITE));
    }

    #[test]
    fn negative_source_width_mirrors() {
        let mut c = canvas_with_probe();
        c.blit(Sheet::Tracks, Rectangle::new(0.0, 0.0, -4.0, 4.0), Rectangle::new(0.0, 0.0, 4.0, 4.0), Vec2::zero(), 0.0, Color::WHITE);
        assert_eq!(c.px(3, 0), rgba(RED), "red texel now on the right");
        assert_eq!(c.px(0, 0), rgba(GREEN));
        assert_eq!(c.px(3, 3), rgba(BLUE));
    }

    #[test]
    fn quarter_turn_about_the_centre() {
        let mut c = canvas_with_probe();
        // Rotating 90 degrees clockwise about the sprite's centre: the
        // top-left texel goes to the top-right.
        c.blit(Sheet::Tracks, Rectangle::new(0.0, 0.0, 4.0, 4.0), Rectangle::new(2.0, 2.0, 4.0, 4.0), Vec2::new(2.0, 2.0), 90.0, Color::WHITE);
        assert_eq!(c.px(3, 0), rgba(RED));
        assert_eq!(c.px(3, 3), rgba(GREEN));
        assert_eq!(c.px(0, 0), rgba(BLUE));
    }

    #[test]
    fn origin_shifts_the_quad() {
        let mut c = canvas_with_probe();
        c.blit(Sheet::Tracks, Rectangle::new(0.0, 0.0, 4.0, 4.0), Rectangle::new(8.0, 8.0, 4.0, 4.0), Vec2::new(2.0, 2.0), 0.0, Color::WHITE);
        assert_eq!(c.px(6, 6), rgba(RED));
        assert_eq!(c.px(9, 6), rgba(GREEN));
    }

    #[test]
    fn tint_and_alpha_blend_over() {
        let mut c = canvas_with_probe();
        // Half-transparent black over white gives mid grey.
        c.fill_rect(0, 0, 2, 2, Color::new(0, 0, 0, 128));
        let p = c.pixel(0, 0);
        assert!((126..=129).contains(&p.r) && p.r == p.g && p.g == p.b, "{p:?}");
        assert_eq!(p.a, 255);
        // A tinted opaque texel is multiplied.
        c.blit(Sheet::Tracks, Rectangle::new(0.0, 0.0, 4.0, 4.0), Rectangle::new(8.0, 0.0, 4.0, 4.0), Vec2::zero(), 0.0, Color::new(128, 255, 255, 255));
        assert_eq!(c.px(8, 0), rgba(Color::new(128, 0, 0, 255)));
    }

    #[test]
    fn fully_transparent_paint_is_a_no_op() {
        let mut c = CpuCanvas::blank(4, 4);
        c.fill_rect(0, 0, 4, 4, CLEAR);
        c.disc(Position::new(2.0, 2.0), 2.0, CLEAR);
        assert!(c.pixels().iter().all(|&p| rgba(p) == rgba(WHITE)));
    }

    #[test]
    fn clipping_never_panics() {
        let mut c = CpuCanvas::blank(8, 8);
        c.fill_rect(-4, -4, 20, 20, RED);
        c.disc(Position::new(-3.0, 4.0), 5.0, GREEN);
        c.ring(Position::new(9.0, 9.0), 1.0, 6.0, 0.0, 360.0, 48, BLUE);
        c.gradient_v(-2, -2, 12, 12, RED, BLUE);
        c.gradient_h(0, 0, 0, 0, RED, BLUE);
        let mut d = canvas_with_probe();
        d.blit(Sheet::Tracks, Rectangle::new(0.0, 0.0, 4.0, 4.0), Rectangle::new(-2.0, -2.0, 40.0, 40.0), Vec2::zero(), 33.0, Color::WHITE);
        d.blit(Sheet::Grass(Theme::Grass), Rectangle::new(0.0, 0.0, 4.0, 4.0), Rectangle::new(0.0, 0.0, 4.0, 4.0), Vec2::zero(), 0.0, Color::WHITE);
    }

    #[test]
    fn disc_and_ring_cover_the_expected_pixels() {
        let mut c = CpuCanvas::blank(16, 16);
        c.disc(Position::new(8.0, 8.0), 3.0, RED);
        assert_eq!(c.px(8, 8), rgba(RED));
        assert_eq!(c.px(8, 5), rgba(RED), "top of the disc");
        assert_eq!(c.px(8, 4), rgba(Color::WHITE));
        assert_eq!(c.px(3, 3), rgba(Color::WHITE));

        let mut c = CpuCanvas::blank(16, 16);
        c.ring(Position::new(8.0, 8.0), 4.0, 6.0, 0.0, 360.0, 48, BLUE);
        assert_eq!(c.px(8, 8), rgba(Color::WHITE), "the hole");
        assert_eq!(c.px(13, 8), rgba(BLUE), "the band to the right");
        assert_eq!(c.px(8, 13), rgba(BLUE), "the band below");
    }

    #[test]
    fn ring_arc_follows_raylib_angles() {
        // raylib angles run from +x clockwise on the y-down screen, so an
        // arc from -90 to 0 covers the top-right quadrant only.
        let mut c = CpuCanvas::blank(16, 16);
        c.ring(Position::new(8.0, 8.0), 4.0, 6.0, -90.0, 0.0, 12, BLUE);
        assert_eq!(c.px(11, 4), rgba(BLUE), "top-right quadrant");
        assert_eq!(c.px(4, 4), rgba(Color::WHITE), "top-left untouched");
        assert_eq!(c.px(4, 11), rgba(Color::WHITE), "bottom-left untouched");
        assert_eq!(c.px(11, 11), rgba(Color::WHITE), "bottom-right untouched");
        // A reversed pair draws the same arc, as raylib swaps them.
        let mut d = CpuCanvas::blank(16, 16);
        d.ring(Position::new(8.0, 8.0), 4.0, 6.0, 0.0, -90.0, 12, BLUE);
        assert_eq!(d.hash(), c.hash());
    }

    #[test]
    fn gradients_run_the_documented_way() {
        let mut c = CpuCanvas::blank(4, 8);
        c.gradient_v(0, 0, 4, 8, Color::new(0, 0, 0, 255), Color::new(0, 0, 0, 0));
        assert!(c.pixel(0, 0).r < c.pixel(0, 7).r, "dark at the top, clear at the bottom");
        let mut c = CpuCanvas::blank(8, 4);
        c.gradient_h(0, 0, 8, 4, Color::new(0, 0, 0, 0), Color::new(0, 0, 0, 255));
        assert!(c.pixel(0, 0).r > c.pixel(7, 0).r, "clear on the left, dark on the right");
    }

    #[test]
    fn hash_changes_with_content_and_is_stable() {
        let a = CpuCanvas::blank(4, 4);
        let mut b = CpuCanvas::blank(4, 4);
        assert_eq!(a.hash(), b.hash());
        b.fill_rect(1, 1, 1, 1, RED);
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn every_sheet_path_is_distinct() {
        let all = Sheet::all();
        let mut paths: Vec<String> = all.iter().map(|s| s.path()).collect();
        let n = paths.len();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), n);
        assert_eq!(n, 2 * Theme::ALL.len() + SINGLE_SHEETS.len() + PICKUP_KINDS.len() + FROG_VARIANT_DIRS.len() * FROG_CLIPS.len());
    }

    /// The ground and grass sheets follow the theme, the way `app.rs`
    /// picks them per frame - a desert map's thumbnail is drawn from the
    /// desert tileset, not the grass one.
    #[test]
    fn ground_and_grass_sheets_follow_the_theme() {
        for theme in Theme::ALL {
            assert_eq!(Sheet::Ground(theme).path(), theme.ground_texture_path());
            assert_eq!(Sheet::Grass(theme).path(), theme.grass_texture_path());
        }
        assert_ne!(Sheet::Ground(Theme::Grass).path(), Sheet::Ground(Theme::Desert).path());
        assert_ne!(Sheet::Grass(Theme::Grass).path(), Sheet::Grass(Theme::Desert).path());
    }

    /// Decoding is raylib's, so this test needs the `render` feature.
    #[cfg(feature = "render")]
    #[test]
    fn every_sheet_loads_from_static() {
        unsafe { sola_raylib::ffi::SetTraceLogLevel(sola_raylib::consts::TraceLogLevel::LOG_WARNING as i32) }
        // Runs with the crate root as the working directory, where `static/`
        // is; this is also the CPU renderer's asset contract.
        for sheet in Sheet::all() {
            let p = Pixels::load(&sheet.path()).unwrap_or_else(|e| panic!("{sheet:?}: {e}"));
            assert!(p.width > 0 && p.height > 0 && p.data.len() == p.width * p.height, "{sheet:?}");
        }
    }
}
