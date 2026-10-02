//! The ripple effects' state: a `Shockwave` per ripple in flight, kept by
//! the round (`Game::shocks`, `Game::muzzle_flashes`), and the camera shake
//! the kill ripples add up to (`camera_shake`). The shader that resolves
//! them is `render::shockwave::RippleFx`.

use crate::math::{Rectangle, Vec2};
use crate::tuning::Tuning;
use crate::Position;

/// The frame every ripple - the kill shockwave, a muzzle flash, a shell
/// impact - is measured in: the standard field, `DEFAULT_SCREEN_WIDTH` x
/// `DEFAULT_SCREEN_HEIGHT`. Their speed, width and strength rows are in its
/// units (x in its widths and y in its heights, made round by its aspect,
/// so a distance is in its heights), which makes a ring the same size in
/// world pixels on every map - an arena drawn whole and a field map the
/// camera follows at a texel per world pixel alike - and keeps the flashes
/// inside the quads they are drawn in.
pub const RIPPLE_FRAME: (f32, f32) = (crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);

/// A world position in the ripple shaders' frame, for a view whose scene
/// target starts at the world point `corner` (`Camera::ripple_view`): x
/// across from it and y up from it - a render texture reads bottom-up -
/// in `RIPPLE_FRAME` units. Measured from the view rather than the field,
/// so the numbers a shader works with stay small on any map, as a GLSL ES
/// `mediump` needs them.
pub fn ripple_uv(corner: Vec2, pos: Position) -> Vec2 {
    Vec2::new((pos.x - corner.x) / RIPPLE_FRAME.0, (corner.y - pos.y) / RIPPLE_FRAME.1)
}

/// A ripple effect in flight: a radial-distortion ring expanding from
/// `center`, `time` seconds after it started. Shared shape for both ripple
/// effects in the game: the full-screen kill shockwave (`Game::shock`, at
/// most one at a time) and the small, split-second muzzle-flash heat haze
/// (`Game::muzzle_flashes`, one per shot fired). See `Game::render`.
pub struct Shockwave {
    /// Hit point in world pixels.
    pub center: Position,
    /// Seconds since the ripple was triggered.
    pub time: f32,
    /// How hard this one hits, as a multiple of `shockwave_strength` and
    /// `camera_shake_magnitude`. 1.0 is a tank dying; a fence collapsing
    /// has no business shaking the screen as hard as that.
    pub strength: f32,
}

impl Shockwave {
    /// A ripple at full strength.
    pub fn new(center: Position) -> Self {
        Shockwave { center, time: 0.0, strength: 1.0 }
    }

    /// A ripple scaled against a tank kill, which is the 1.0 reference.
    pub fn scaled(center: Position, strength: f32) -> Self {
        Shockwave { center, time: 0.0, strength }
    }

    /// Punch left in it: strength faded by how much of its life is gone.
    /// `Game::finish_frame` evicts by this rather than by age, so a barrel
    /// cascade's little fuse pops cannot shove out the tank explosion that
    /// set them off.
    pub fn remaining(&self) -> f32 {
        let left = 1.0 - (self.time / crate::tuning::tuning().shockwave_duration).clamp(0.0, 1.0);
        self.strength * left
    }
}

/// The camera shake `shocks` add up to this frame, in world pixels: how far
/// the finished scene is shifted when it lands on the bitmap. A short
/// decaying wobble per ripple (`camera_shake_duration`,
/// `camera_shake_magnitude` times its strength and `screen_fx_intensity`),
/// two out-of-phase sines standing in for randomness so x and y do not
/// shake in lockstep, each phased off its own position hash so several
/// overlapping shakes interfere instead of doubling cleanly - a pure
/// function of the round, no RNG. The sum is capped at
/// `camera_shake_max_stack` times the magnitude, or three kills at once
/// throw the scene far enough that the screen edge shows through, and
/// snapped to whole 2 px blocks: at a fractional offset every pixel of the
/// scene samples between texels and the whole screen shimmers, where a
/// whole block reads as a hard jolt.
///
/// `view` is the world rectangle the screen shows when that is less than
/// the whole field (docs/large-maps-follow-camera.md section 6): each
/// ripple then shakes it by `shake_reach` - fully near the view, not at all
/// from a few screens away - so a blast across a field map does not shake
/// a screen that cannot see it. `None` is the whole field, which every
/// ripple is in: an arena's shake, exactly as it always was.
pub fn camera_shake(shocks: &[Shockwave], view: Option<Rectangle>, t: &Tuning) -> Vec2 {
    let mut offset = Vec2::new(0.0, 0.0);
    // `screen_fx_intensity` scales every whole-screen effect together;
    // folding it into the magnitude keeps the stack cap proportional.
    let magnitude = t.camera_shake_magnitude * t.screen_fx_intensity;
    for shock in shocks {
        let decay = (1.0f32 - shock.time / t.camera_shake_duration).max(0.0);
        if decay <= 0.0 {
            continue;
        }
        let mut mag = magnitude * shock.strength * decay;
        if let Some(view) = view {
            mag *= shake_reach(shock.center, view, t);
        }
        let phase = (crate::blast::seed_for(shock.center) % 628) as f32 * 0.01;
        let wobble = shock.time * t.camera_shake_frequency + phase;
        offset.x += wobble.sin() * mag;
        offset.y += (wobble * 1.3 + 1.7).sin() * mag;
    }
    let cap = magnitude * t.camera_shake_max_stack;
    let len = (offset.x * offset.x + offset.y * offset.y).sqrt();
    if len > cap && len > 0.0 {
        offset.x *= cap / len;
        offset.y *= cap / len;
    }
    offset.x = (offset.x / 2.0).round() * 2.0;
    offset.y = (offset.y / 2.0).round() * 2.0;
    offset
}

/// How much of a ripple at `at` shakes a screen showing the world
/// rectangle `view`: all of it anywhere within `camera_shake_margin_px` of
/// the view - a blast just off screen still throws its fire into it -
/// fading to nothing `camera_shake_fade_screens` screens past that, a
/// screen being the view's width across and its height up and down
/// (surviv.io's rule: full shake near the camera, none far from it).
pub fn shake_reach(at: Position, view: Rectangle, t: &Tuning) -> f32 {
    let margin = t.camera_shake_margin_px.max(0.0);
    let dx = (view.x - margin - at.x).max(at.x - (view.x + view.width + margin)).max(0.0);
    let dy = (view.y - margin - at.y).max(at.y - (view.y + view.height + margin)).max(0.0);
    if dx <= 0.0 && dy <= 0.0 {
        return 1.0;
    }
    let fade = t.camera_shake_fade_screens;
    if !(fade > 0.0) {
        return 0.0;
    }
    let screens = ((dx / view.width.max(1.0)).powi(2) + (dy / view.height.max(1.0)).powi(2)).sqrt();
    (1.0 - screens / fade).clamp(0.0, 1.0)
}

#[cfg(test)]
mod shake_tests {
    use super::*;

    /// The shake as `Game::render` worked it out before it read the view:
    /// every ripple at full reach. An arena's shake must stay this, bit for
    /// bit.
    fn unattenuated(shocks: &[Shockwave], t: &Tuning) -> Vec2 {
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let shake_magnitude = t.camera_shake_magnitude * t.screen_fx_intensity;
        for shock in shocks {
            let decay = (1.0f32 - shock.time / t.camera_shake_duration).max(0.0);
            if decay <= 0.0 {
                continue;
            }
            let mag = shake_magnitude * shock.strength * decay;
            let phase = (crate::blast::seed_for(shock.center) % 628) as f32 * 0.01;
            let w = shock.time * t.camera_shake_frequency + phase;
            x += w.sin() * mag;
            y += (w * 1.3 + 1.7).sin() * mag;
        }
        let cap = shake_magnitude * t.camera_shake_max_stack;
        let len = (x * x + y * y).sqrt();
        if len > cap && len > 0.0 {
            x *= cap / len;
            y *= cap / len;
        }
        Vec2::new((x / 2.0).round() * 2.0, (y / 2.0).round() * 2.0)
    }

    fn shock(x: f32, y: f32, time: f32, strength: f32) -> Shockwave {
        Shockwave { center: Position::new(x, y), time, strength }
    }

    /// Kills at many moments of their shake, alone and stacked, so the
    /// sines sweep both signs and the cap binds.
    fn sets() -> Vec<Vec<Shockwave>> {
        let mut out = Vec::new();
        for i in 0..40 {
            let time = i as f32 * 0.006;
            out.push(vec![shock(300.0 + 13.0 * i as f32, 200.0, time, 1.0)]);
            out.push(vec![shock(100.0, 400.0, time, 1.0), shock(900.0, 120.0, time * 0.5, 0.6), shock(640.0, 300.0, 0.01, 1.0)]);
        }
        out
    }

    #[test]
    fn the_rows_come_from_the_fx_group() {
        for name in ["camera_shake_margin_px", "camera_shake_fade_screens"] {
            assert_eq!(Tuning::meta(name).map(|m| m.group), Some("fx"), "{name}");
        }
        let t = Tuning::DEFAULT;
        assert!(t.camera_shake_margin_px > 0.0 && t.camera_shake_fade_screens > 0.0);
    }

    #[test]
    fn an_arena_shakes_exactly_as_before() {
        // The whole field is no view to attenuate by: every ripple shakes
        // the screen as it always did, bit for bit - and a view that is
        // the whole field gives the same, since every ripple is in it.
        let t = Tuning::DEFAULT;
        let field = Rectangle::new(0.0, 0.0, 1088.0, 544.0);
        let mut moved = 0;
        for shocks in sets() {
            let before = unattenuated(&shocks, &t);
            assert_eq!(camera_shake(&shocks, None, &t), before);
            assert_eq!(camera_shake(&shocks, Some(field), &t), before);
            moved += usize::from(before != Vec2::new(0.0, 0.0));
        }
        assert!(moved > 20, "the sets shake: {moved}");
    }

    #[test]
    fn a_ripple_in_view_or_just_past_it_shakes_fully() {
        let t = Tuning::DEFAULT;
        let view = Rectangle::new(1000.0, 600.0, 1280.0, 720.0);
        // Inside, on the edge and a little past it.
        for (x, y) in [(1500.0, 900.0), (1000.0, 600.0), (2280.0 + t.camera_shake_margin_px, 900.0), (1500.0, 600.0 - 100.0)] {
            assert_eq!(shake_reach(Position::new(x, y), view, &t), 1.0, "{x}, {y}");
        }
        for i in 0..40 {
            let shocks = vec![shock(1400.0, 1000.0, i as f32 * 0.006, 1.0)];
            assert_eq!(camera_shake(&shocks, Some(view), &t), unattenuated(&shocks, &t));
        }
    }

    #[test]
    fn a_ripple_far_from_the_view_shakes_it_not_at_all() {
        let t = Tuning::DEFAULT;
        let view = Rectangle::new(0.0, 0.0, 1280.0, 720.0);
        let past = view.x + view.width + t.camera_shake_margin_px;
        // A blast across a field map, past the fade: nothing.
        let far = past + view.width * t.camera_shake_fade_screens + 1.0;
        assert_eq!(shake_reach(Position::new(far, 300.0), view, &t), 0.0);
        for i in 0..40 {
            let shocks = vec![shock(far, 300.0, i as f32 * 0.006, 1.0)];
            assert_eq!(camera_shake(&shocks, Some(view), &t), Vec2::new(0.0, 0.0));
        }
        // Between, it fades with the screens away, half way at half.
        let half = shake_reach(Position::new(past + view.width * t.camera_shake_fade_screens * 0.5, 300.0), view, &t);
        assert!((half - 0.5).abs() < 1e-4, "{half}");
        let mut last = 1.0;
        for k in 0..=20 {
            let r = shake_reach(Position::new(past + view.width * t.camera_shake_fade_screens * k as f32 / 20.0, 300.0), view, &t);
            assert!(r <= last, "{k}: {r} after {last}");
            last = r;
        }
        // Up and down a screen is the view's height.
        let below = shake_reach(Position::new(640.0, 720.0 + t.camera_shake_margin_px + 720.0 * t.camera_shake_fade_screens * 0.5), view, &t);
        assert!((below - 0.5).abs() < 1e-4, "{below}");
        // A near blast and a far one: only the near one shakes.
        let both = vec![shock(600.0, 300.0, 0.03, 1.0), shock(far, 300.0, 0.05, 1.0)];
        assert_eq!(camera_shake(&both, Some(view), &t), unattenuated(&both[..1], &t));
        // No fade at all: past the margin is nothing.
        let hard = Tuning { camera_shake_fade_screens: 0.0, ..t };
        assert_eq!(shake_reach(Position::new(past + 1.0, 300.0), view, &hard), 0.0);
    }
}
