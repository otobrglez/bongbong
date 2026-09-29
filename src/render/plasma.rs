//! Drawing a plasma bolt - its runtime glow halo and the baked sprite -
//! and its shadow (`plasma.rs` owns the entity and the frame arithmetic).

use sola_raylib::prelude::*;

use crate::math::{Color, Rectangle};
use crate::plasma::{Plasma, PlasmaState, PlasmaVariant};
use crate::render::blast::pixel_disc;
use crate::render::shot_fx::{fade, heading, pixel_streak};
use crate::tuning::tuning;
use crate::{Position, PLASMA_SCALE, PLASMA_TEXTURE_SIZE};

impl PlasmaVariant {
    /// Which row of plasma.png this variant draws from - a genuine second
    /// colour pass (see docs/PLASMA_SPEC.md), not a runtime tint over one
    /// shared row: `draw_texture_pro`'s tint is a per-channel multiply, which
    /// can only ever darken/filter a pixel toward the tint colour, never
    /// invert a channel that started at zero - tinting the (zero-red) teal
    /// glow body purple was tried and just produced a darker blue with
    /// purple-tinted white highlights, not a purple bolt.
    fn row(self) -> i32 {
        match self {
            PlasmaVariant::Teal => 0,
            PlasmaVariant::Purple => 1,
        }
    }

    /// (outer, inner) glow-halo colours at full alpha, for
    /// `draw_plasma_light`'s halo and trail - drawn fresh each frame (not sampled from
    /// the sprite), matched to this variant's own baked row
    /// (`tools/spritegen/gen_plasma.py`'s `TEAL`/`PURPLE` palettes) so the
    /// halo and the sprite read as the same colour.
    fn glow_colors(self) -> (Color, Color) {
        match self {
            PlasmaVariant::Teal => (Color::new(40, 220, 200, 255), Color::new(200, 255, 245, 255)),
            PlasmaVariant::Purple => {
                (Color::new(155, 77, 224, 255), Color::new(230, 205, 255, 255))
            }
        }
    }
}

/// Column of this state in plasma.png - never asked for `Flying`, whose
/// four columns `flying_col` cycles through instead.
fn state_col(state: PlasmaState) -> i32 {
    state.col()
}

/// Source rectangle for a plasma frame at sheet column `col`, row
/// `variant.row()` in plasma.png - no chassis variants (see `plasma.rs`'s
/// doc comment), just the two `PlasmaVariant` rows.
fn source_rec(col: i32, variant: PlasmaVariant) -> Rectangle {
    Rectangle::new(
        col as f32 * PLASMA_TEXTURE_SIZE,
        variant.row() as f32 * PLASMA_TEXTURE_SIZE,
        PLASMA_TEXTURE_SIZE,
        PLASMA_TEXTURE_SIZE,
    )
}

/// Which of `Flying`'s 4 baked breathing-cycle columns (3, 4, 5, 6) to draw
/// right now, from `plasma.timer` (see its doc comment) - cycles forward at
/// PLASMA_FLYING_CYCLE_FPS, wrapping every 4 frames. A plain forward cycle
/// (0,1,2,3,0,1,2,...), not a phase-matched sine like the runtime glow's own
/// `glow_pulse` - `gen_plasma.py`'s 4 frames are themselves authored as a
/// dim->bright->dim breathing loop (see docs/PLASMA_SPEC.md), so a steady
/// cycle through them already reads as pulsing without needing to sample a
/// continuous curve.
fn flying_col(timer: f32) -> i32 {
    3 + (timer * tuning().plasma_flying_cycle_fps) as i32 % 4
}

/// The in-flight glow halo's current radius/alpha, derived from `timer`
/// (see its doc comment) - a sine wave over PLASMA_PULSE_HZ cycles/second,
/// remapped from the base sprite's own half-size into
/// PLASMA_PULSE_MIN_SCALE..MAX_SCALE. Shared by `draw_plasma_light`'s halo
/// passes so they always pulse in lockstep. Deliberately a different cycle
/// rate/shape than the baked `flying_col` animation - see
/// PLASMA_FLYING_CYCLE_FPS's doc comment.
fn glow_pulse(plasma: &Plasma) -> (f32, f32) {
    let base_radius = PLASMA_TEXTURE_SIZE * PLASMA_SCALE * 0.5;
    let phase = (plasma.timer * tuning().plasma_pulse_hz * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let scale = tuning().plasma_pulse_min_scale + (tuning().plasma_pulse_max_scale - tuning().plasma_pulse_min_scale) * phase;
    (base_radius * scale, phase)
}

impl PlasmaVariant {
    /// The orb's four shading steps, dark rim to white-hot highlight, in
    /// the same family as this variant's baked row and glow.
    fn orb_colors(self) -> [Color; 4] {
        match self {
            PlasmaVariant::Teal => [
                Color::new(10, 78, 98, 255),
                Color::new(24, 164, 176, 255),
                Color::new(110, 236, 222, 255),
                Color::new(236, 255, 250, 255),
            ],
            PlasmaVariant::Purple => [
                Color::new(58, 22, 104, 255),
                Color::new(128, 60, 204, 255),
                Color::new(196, 144, 255, 255),
                Color::new(248, 234, 255, 255),
            ],
        }
    }
}

/// How far the orb has spun, in radians: off the round clock plus a
/// per-bolt phase, so a replica (whose bolts' timers do not run) spins
/// its orbs too and two bolts in a volley do not turn in step.
fn spin(plasma: &Plasma, time: f32) -> f32 {
    let phase = (plasma.id.wrapping_mul(2_654_435_761) >> 20) as f32 / 4096.0 * std::f32::consts::TAU;
    time * tuning().plasma_orb_spin_hz * std::f32::consts::TAU + phase
}

/// One of the orb's two orbit rings: its radius as a multiple of the
/// orb's, how flat it lies (the minor/major axis ratio), the screen angle
/// of its major axis and its speed relative to the spin (negative runs
/// the other way).
struct Ring {
    radius: f32,
    flat: f32,
    tilt: f32,
    speed: f32,
}

const RINGS: [Ring; 2] = [
    Ring { radius: 1.95, flat: 0.3, tilt: -0.45, speed: 1.0 },
    Ring { radius: 1.6, flat: 0.4, tilt: 0.95, speed: -1.4 },
];

/// Walk a ring and draw the half of it that is behind the orb (`front`
/// false) or in front of it (`front` true): a dotted ellipse of single
/// blocks, the far half in `far` and the near half in `near`, and two
/// motes riding it with a short tail of dots behind them.
fn draw_ring(d: &mut impl RaylibDraw, center: Position, r: f32, ring: &Ring, turn: f32, front: bool, [far, near, mote]: [Color; 3]) {
    let big = r * ring.radius;
    let (ct, st) = (ring.tilt.cos(), ring.tilt.sin());
    let steps = ((std::f32::consts::TAU * big) / 2.0).ceil().max(8.0) as i32;
    let place = |a: f32| {
        let (x, y) = (a.cos() * big, a.sin() * big * ring.flat);
        Position::new(center.x + x * ct - y * st, center.y + x * st + y * ct)
    };
    // The lower half of the ellipse (sin > 0) is the near side.
    let near_side = |a: f32| a.rem_euclid(std::f32::consts::TAU).sin() > 0.0;
    for i in 0..steps {
        let a = i as f32 / steps as f32 * std::f32::consts::TAU;
        if near_side(a) == front {
            pixel_disc_or_block(d, place(a), 0.0, if front { near } else { fade(far, 0.7) });
        }
    }
    for m in 0..2 {
        let head = turn * ring.speed + m as f32 * std::f32::consts::PI;
        let back = -ring.speed.signum() * 0.22;
        for (k, a) in [head, head + back, head + back * 2.0].into_iter().enumerate() {
            if near_side(a) == front {
                let c = if k == 0 { mote } else { fade(near, 1.0 - 0.3 * k as f32) };
                pixel_disc_or_block(d, place(a), 0.0, if front { c } else { fade(c, 0.6) });
            }
        }
    }
}

/// A single block at `at`, or a block-built disc when `radius` is at
/// least one block.
fn pixel_disc_or_block(d: &mut impl RaylibDraw, at: Position, radius: f32, color: Color) {
    if radius >= 2.0 {
        pixel_disc(d, at, radius, color);
    } else {
        let x = (at.x / 2.0).floor() * 2.0;
        let y = (at.y / 2.0).floor() * 2.0;
        d.draw_rectangle(x as i32, y as i32, 2, 2, color);
    }
}

/// Draw a plasma bolt. In flight it is the orb, a "magic" sphere drawn
/// in true colour (normal blend, so the variant's hue survives any
/// ground): a comet tail of shrinking afterimages and a halo pulsing
/// with `glow_pulse`, the far halves of two tilted orbit rings, the sphere shaded
/// from a dark rim to a white-hot highlight up and to the left - stacked
/// block discs, each nudged toward the light, which is what reads as
/// round -, two energy bands turning on its face, then the rings' near
/// halves with their motes. `draw_plasma_light` and `draw_plasma_front`
/// add the light around it.
/// Every other state (the muzzle burst, the impact splash) is the baked
/// sprite from `plasma.variant`'s row of plasma.png, centred and rotated
/// to face travel like `draw_shell`/`draw_bullet`.
pub fn draw_plasma(d: &mut impl RaylibDraw, texture: &Texture2D, plasma: &Plasma, time: f32) {
    if plasma.state == PlasmaState::Flying && tuning().shot_glow_strength > 0.0 {
        let r = tuning().plasma_orb_radius;
        let [rim, body, bright, hot] = plasma.variant.orb_colors();
        let at = plasma.position;
        // The comet tail and the halo, as tinted paint rather than light:
        // added light would wash the purple to grey over grass.
        let (_, phase) = glow_pulse(plasma);
        let (glow_outer, _) = plasma.variant.glow_colors();
        let dir = heading(plasma.rotation);
        let trail = tuning().plasma_trail_length;
        const GHOSTS: i32 = 5;
        for i in (1..=GHOSTS).rev() {
            let t = i as f32 / GHOSTS as f32;
            let ghost = Position::new(at.x - dir.x * trail * t, at.y - dir.y * trail * t);
            pixel_disc(d, ghost, r * (1.0 - 0.65 * t), fade(glow_outer, 0.5 * (1.0 - t)));
        }
        pixel_disc(d, at, r * (1.45 + 0.25 * phase), fade(glow_outer, 0.22 + 0.18 * phase));
        let turn = spin(plasma, time);
        for ring in &RINGS {
            draw_ring(d, at, r, ring, turn, false, [body, bright, hot]);
        }
        let toward_light = |k: f32| Position::new(at.x - r * k, at.y - r * k);
        pixel_disc(d, at, r, rim);
        pixel_disc(d, toward_light(0.1), r * 0.82, body);
        pixel_disc(d, toward_light(0.26), r * 0.5, fade(bright, 0.8));
        pixel_disc(d, toward_light(0.4), r * 0.2, hot);
        // Two energy bands, great circles on the sphere tilted `lean` off
        // the equator and spun about the vertical axis; only the points
        // facing us are drawn, so they read as wrapped round a ball.
        for (lean, speed) in [(0.55_f32, 1.0_f32), (-1.1, 0.7)] {
            let steps = 30;
            for i in 0..steps {
                let u = i as f32 / steps as f32 * std::f32::consts::TAU;
                let (x, y, z) = (u.cos(), u.sin() * lean.cos(), u.sin() * lean.sin());
                let s = turn * speed;
                let (xs, zs) = (x * s.cos() + z * s.sin(), -x * s.sin() + z * s.cos());
                if zs <= 0.15 {
                    continue;
                }
                let p = Position::new(at.x + xs * r * 0.8, at.y + y * r * 0.8);
                pixel_disc_or_block(d, p, 0.0, if zs > 0.75 { hot } else { fade(bright, 0.4 + 0.6 * zs) });
            }
        }
        for ring in &RINGS {
            draw_ring(d, at, r, ring, turn, true, [body, bright, hot]);
        }
        return;
    }
    let col = if plasma.state == PlasmaState::Flying {
        flying_col(plasma.timer)
    } else {
        state_col(plasma.state)
    };
    let src = source_rec(col, plasma.variant);
    let size = PLASMA_TEXTURE_SIZE * PLASMA_SCALE;
    let dest = Rectangle::new(plasma.position.x, plasma.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, src, dest, origin, plasma.rotation, Color::WHITE);
}

/// A flying bolt's light behind the orb (call inside the additive block,
/// before the sprites): the white-hot streak down the middle of its
/// comet tail (`plasma_trail_length`).
pub fn draw_plasma_light(d: &mut impl RaylibDraw, plasma: &Plasma) {
    let strength = tuning().shot_glow_strength;
    if plasma.state != PlasmaState::Flying || strength <= 0.0 {
        return;
    }
    let (glow_outer, glow_inner) = plasma.variant.glow_colors();
    let dir = heading(plasma.rotation);
    let trail = tuning().plasma_trail_length;
    pixel_streak(d, plasma.position, dir, trail, 1, fade(glow_inner, strength * 0.9), fade(glow_outer, 0.0));
}

/// The light in front of a flying orb (call inside the additive block
/// drawn after the sprites): a hot core glow pulsing on the highlight, a
/// hard glint, and a flash on each ring mote as it swings to the front.
pub fn draw_plasma_front(d: &mut impl RaylibDraw, plasma: &Plasma, time: f32) {
    let strength = tuning().shot_glow_strength;
    if plasma.state != PlasmaState::Flying || strength <= 0.0 {
        return;
    }
    let r = tuning().plasma_orb_radius;
    let at = plasma.position;
    let (_, phase) = glow_pulse(plasma);
    let (_, glow_inner) = plasma.variant.glow_colors();
    let [_, _, _, hot] = plasma.variant.orb_colors();
    let highlight = Position::new(at.x - r * 0.3, at.y - r * 0.3);
    pixel_disc(d, highlight, r * (0.3 + 0.1 * phase), fade(glow_inner, strength * 0.3));
    pixel_disc_or_block(d, Position::new(at.x - r * 0.42, at.y - r * 0.42), 0.0, fade(hot, strength));
    let turn = spin(plasma, time);
    for ring in &RINGS {
        let big = r * ring.radius;
        let (ct, st) = (ring.tilt.cos(), ring.tilt.sin());
        for m in 0..2 {
            let a = (turn * ring.speed + m as f32 * std::f32::consts::PI).rem_euclid(std::f32::consts::TAU);
            let depth = a.sin();
            if depth > 0.5 {
                let (x, y) = (a.cos() * big, a.sin() * big * ring.flat);
                let p = Position::new(at.x + x * ct - y * st, at.y + x * st + y * ct);
                pixel_disc(d, p, 3.0, fade(glow_inner, strength * (depth - 0.5) * 1.4));
            }
        }
    }
}

/// Draw this bolt's drop shadow - same tint/offset convention as
/// `draw_shell_shadow`, no glow halo (a shadow is a flat silhouette, not a
/// light source) - still reads from `plasma.variant`'s own row since the two
/// rows aren't pixel-identical (unlike shells.png's Flying column), just
/// drawn as a flat black silhouette regardless of colour, same as every
/// other shadow pass in the game. Caller (`Game::render`) only calls this
/// while `plasma.state == PlasmaState::Flying`.
pub fn draw_plasma_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, plasma: &Plasma) {
    if tuning().shot_glow_strength > 0.0 {
        // The orb's own round silhouette, not the baked sprite's.
        let at = Position::new(
            plasma.position.x + tuning().shadow_dir_x * plasma.shadow_offset,
            plasma.position.y + tuning().shadow_dir_y * plasma.shadow_offset,
        );
        let a = (255.0 * tuning().plasma_shadow_opacity) as u8;
        pixel_disc(d, at, tuning().plasma_orb_radius, Color::new(0, 0, 0, a));
        return;
    }
    let src = source_rec(flying_col(plasma.timer), plasma.variant);
    let size = PLASMA_TEXTURE_SIZE * PLASMA_SCALE;
    let dest = Rectangle::new(
        plasma.position.x + tuning().shadow_dir_x * plasma.shadow_offset,
        plasma.position.y + tuning().shadow_dir_y * plasma.shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().plasma_shadow_opacity) as u8);
    d.draw_texture_pro(texture, src, dest, origin, plasma.rotation, shadow);
}
