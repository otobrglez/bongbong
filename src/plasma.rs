//! The plasma cannon's projectile (`Plasma`/`PlasmaState`): a limited-ammo
//! upgrade over a tank's traditional shell (see
//! `pickup::PickupKind::Plasma`, `Tank::plasma_ammo`), fired the exact same
//! way - straight down the barrel, a twin-barrel chassis firing one bolt per
//! barrel a beat apart (see `Tank::pending_plasma_shot`, mirroring
//! `shell::Shell`'s `PendingShot`) - but dealing PLASMA_DAMAGE_FACTOR more
//! damage and rendered as a glowing, pulsating orb (the runtime sine-wave
//! glow in `draw_plasma`, layered on top of a 4-frame baked breathing
//! animation while `Flying` - see `flying_col`/docs/PLASMA_SPEC.md) that
//! bursts into a small electric/sci-fi splash on impact instead of a
//! shell's smoke-and-fire blast.
//!
//! Comes in two `PlasmaVariant`s, Teal (base) and Purple (rarer, hits
//! harder) - see `pickup::PickupKind::Plasma`'s pickup-time reroll, the same
//! mechanism `laser::LaserVariant` already uses.
//!
//! Mirrors `shell::Shell`'s shape (position/velocity/rotation/timer/owner/
//! shadow_offset/prev_position) and its 7-state Fire/Flying/Hit choreography,
//! but - like `bullet::Bullet` - carries no chassis-matched sprite row (the
//! sheet's two rows are `PlasmaVariant`, a property of the ammo, not the
//! shooter chassis - every tank's bolts of the same variant look identical,
//! `bullet.rs`'s "shared art regardless of shooter" convention) and never
//! ricochets (no `bounces_left`): a heavy plasma bolt detonates on first
//! contact rather than bouncing off Iron/walls the way a shell can.

use crate::tuning::tuning;
use sola_raylib::prelude::*;

use crate::shell::Owner;
use crate::tank::Tank;
use crate::{
    PLASMA_SCALE,
    PLASMA_TEXTURE_SIZE,
    Position,
};

/// A plasma bolt's lifecycle - same overall Fire/Flying/Hit shape as
/// `shell::ShellState`, just its own sheet (`static/plasma.png`) and its own
/// column layout (see docs/PLASMA_SPEC.md): `Flying` alone spans 4 columns
/// (see `flying_col`) rather than one, for a baked breathing animation
/// instead of a single static frame.
#[derive(Clone, Copy, PartialEq)]
pub enum PlasmaState {
    Fire0,  // col 0 - charge building at the muzzle
    Fire1,  // col 1 - bright flash as the bolt clears the barrel
    Fire2,  // col 2 - flash finishing, bolt pulling away
    Flying, // cols 3-6 (see `flying_col`) - glowing, breathing orb in the air
    Hit0,   // col 7 - impact burst starting
    Hit1,   // col 8 - electric burst expanding, arcs radiating outward
    Hit2,   // col 9 - burst dissipating
}

impl PlasmaState {
    /// Column of this state in plasma.png - meaningless for `Flying`
    /// (superseded by `flying_col`'s 4-frame cycle), kept here only so this
    /// match stays exhaustive; nothing calls it for that variant.
    fn col(self) -> i32 {
        match self {
            PlasmaState::Fire0 => 0,
            PlasmaState::Fire1 => 1,
            PlasmaState::Fire2 => 2,
            PlasmaState::Flying => 3,
            PlasmaState::Hit0 => 7,
            PlasmaState::Hit1 => 8,
            PlasmaState::Hit2 => 9,
        }
    }

    /// How long this state is shown (seconds) - identical timings to
    /// `ShellState::duration`, since a plasma bolt fires at the same cadence
    /// a shell does (unlike a minigun burst, there's no reason for its
    /// choreography to read any faster or slower).
    fn duration(self) -> f32 {
        match self {
            PlasmaState::Fire0 => 0.06,
            PlasmaState::Fire1 => 0.06,
            PlasmaState::Fire2 => 0.05,
            PlasmaState::Flying => f32::INFINITY,
            PlasmaState::Hit0 => 0.08,
            PlasmaState::Hit1 => 0.1,
            PlasmaState::Hit2 => 0.14,
        }
    }
}

/// Which colour batch a plasma charge is - rolled once per
/// `pickup::PickupKind::Plasma` pickup (see `PLASMA_PURPLE_PICKUP_CHANCE`)
/// and carried on `Tank::plasma_variant` until the next pickup rerolls it,
/// same mechanism as `laser::LaserVariant`/`Tank::laser_variant`. Teal is
/// the original, baseline-damage bolt; Purple is `PLASMA_PURPLE_DAMAGE_FACTOR`
/// times stronger, on top of the base plasma bolt's own
/// `PLASMA_DAMAGE_FACTOR` over a shell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlasmaVariant {
    Teal,
    Purple,
}

impl PlasmaVariant {
    /// Multiplier on this bolt's already-boosted `PLASMA_DAMAGE_FACTOR`
    /// damage.
    pub fn damage_factor(self) -> f32 {
        match self {
            PlasmaVariant::Teal => 1.0,
            PlasmaVariant::Purple => tuning().plasma_purple_damage_factor,
        }
    }

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

    /// (outer, inner) glow-halo colours at full alpha, for `draw_plasma`'s
    /// two runtime-drawn circles - drawn fresh each frame (not sampled from
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

/// One plasma bolt - see this module's doc comment for how it compares to
/// `shell::Shell`/`bullet::Bullet`.
pub struct Plasma {
    pub state: PlasmaState,
    pub position: Position,
    /// Direction of travel while flying (pixels per second).
    pub velocity: Vector2,
    /// Facing angle in degrees (matches the tank's rotation when fired).
    pub rotation: f32,
    /// Time elapsed in the current state - kept growing (unbounded) while
    /// `Flying`, same trick `Shell`/`Bullet` already rely on (their own
    /// `Flying::duration()` being infinite means the increment-then-return
    /// in `update` never resets it) - `draw_plasma` reads this to phase both
    /// the runtime glow pulse and the baked `flying_col` breathing cycle.
    pub timer: f32,
    /// Set once the bolt has finished its last state and can be removed.
    pub done: bool,
    /// Who fired this bolt; see `shell::Owner`.
    pub owner: Owner,
    /// Which `PlasmaVariant` the shooter's charge batch was at fire time -
    /// copied here for the same out-of-scope reason as `shooter_row` (the
    /// firing tank is gone by the time this bolt resolves a hit). Drives
    /// both this bolt's damage factor and its draw-time tint/glow colour.
    pub variant: PlasmaVariant,
    /// The firing tank's `row` (0..TANK_VARIANTS), copied at spawn - used to
    /// scale damage by chassis class (TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW),
    /// same as `Shell::shooter_row`/`Bullet::shooter_row`.
    pub shooter_row: i32,
    /// This bolt's drop-shadow distance (px), rolled once at fire time -
    /// same role as `Shell::shadow_offset`.
    pub shadow_offset: f32,
    /// Same role as `Shell::prev_position` - the start of this frame's
    /// swept hit segment, written by the simulation.
    pub prev_position: Position,
}

impl Plasma {
    /// Create a plasma bolt at the tank's muzzle, travelling in the
    /// direction the tank faces - identical muzzle/lateral-offset math to
    /// `Shell::spawn` (see its doc comment for the details), just at
    /// `PLASMA_SPEED` instead of `SHELL_SPEED` and with no `bounces_left` to
    /// set. `variant` is the shooter's current `Tank::plasma_variant`.
    pub fn spawn(
        tank: &Tank,
        owner: Owner,
        variant: PlasmaVariant,
        aim_offset: f32,
        lateral_offset: f32,
    ) -> Plasma {
        let rot = (tank.rotation + aim_offset).to_radians();
        let dir = Vector2::new(rot.sin(), -rot.cos());
        let muzzle = tuning().tank_muzzle_forward_offset[tank.row as usize] * tank.scale;
        let hull_rot = tank.rotation.to_radians();
        let lateral = Vector2::new(hull_rot.cos(), hull_rot.sin()) * (lateral_offset * tank.scale);
        let position = Position::new(
            tank.position.x + dir.x * muzzle + lateral.x,
            tank.position.y + dir.y * muzzle + lateral.y,
        );
        Plasma {
            state: PlasmaState::Fire0,
            position,
            velocity: Vector2::new(dir.x * tuning().plasma_speed, dir.y * tuning().plasma_speed),
            rotation: tank.rotation + aim_offset,
            timer: 0.0,
            done: false,
            owner,
            variant,
            shooter_row: tank.row,
            shadow_offset: 0.0,
            prev_position: position,
        }
    }

    /// Advance the bolt: move it while flying, and step through its timed
    /// states. Mirrors `Shell::update` exactly, just over `PlasmaState`.
    pub fn update(&mut self, dt: f32) {
        self.timer += dt;

        if self.state == PlasmaState::Flying {
            self.position.x += self.velocity.x * dt;
            self.position.y += self.velocity.y * dt;
            return;
        }

        if self.timer >= self.state.duration() {
            self.timer = 0.0;
            self.state = match self.state {
                PlasmaState::Fire0 => PlasmaState::Fire1,
                PlasmaState::Fire1 => PlasmaState::Fire2,
                PlasmaState::Fire2 => PlasmaState::Flying,
                PlasmaState::Flying => PlasmaState::Flying, // handled above
                PlasmaState::Hit0 => PlasmaState::Hit1,
                PlasmaState::Hit1 => PlasmaState::Hit2,
                PlasmaState::Hit2 => {
                    self.done = true;
                    PlasmaState::Hit2
                }
            };
        }
    }

    /// Switch a flying bolt into its impact (hit) animation at the current spot.
    pub fn detonate(&mut self) {
        self.state = PlasmaState::Hit0;
        self.timer = 0.0;
    }
}

/// Source rectangle for a plasma frame at sheet column `col`, row
/// `variant.row()` in plasma.png - no chassis variants (see this module's
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
/// PLASMA_PULSE_MIN_SCALE..MAX_SCALE. Shared by `draw_plasma`'s two glow
/// passes so they always pulse in lockstep. Deliberately a different cycle
/// rate/shape than the baked `flying_col` animation - see
/// PLASMA_FLYING_CYCLE_FPS's doc comment.
fn glow_pulse(plasma: &Plasma) -> (f32, f32) {
    let base_radius = PLASMA_TEXTURE_SIZE * PLASMA_SCALE * 0.5;
    let phase = (plasma.timer * tuning().plasma_pulse_hz * std::f32::consts::TAU).sin() * 0.5 + 0.5;
    let scale = tuning().plasma_pulse_min_scale + (tuning().plasma_pulse_max_scale - tuning().plasma_pulse_min_scale) * phase;
    (base_radius * scale, phase)
}

/// Draw a plasma bolt: while flying, a pulsating glow halo (two concentric
/// translucent discs, sized/faded by `glow_pulse`, coloured by
/// `PlasmaVariant::glow_colors`) drawn first so the sprite composites on top
/// of it, then the sprite itself from its current frame (`flying_col` while
/// `Flying`, `PlasmaState::col` otherwise) at `plasma.variant`'s own sheet
/// row - centered and rotated to face travel, same as
/// `draw_shell`/`draw_bullet`. The glow is purely a runtime draw effect
/// (like `laser::draw_laser_beam`'s fade), layered on top of the sprite's
/// own baked breathing animation rather than replacing it.
pub fn draw_plasma(d: &mut impl RaylibDraw, texture: &Texture2D, plasma: &Plasma) {
    if plasma.state == PlasmaState::Flying {
        let (radius, phase) = glow_pulse(plasma);
        let (glow_outer, glow_inner) = plasma.variant.glow_colors();
        let outer_alpha = (90.0 + 90.0 * phase) as u8;
        d.draw_circle_v(plasma.position, radius, Color::new(glow_outer.r, glow_outer.g, glow_outer.b, outer_alpha));
        d.draw_circle_v(
            plasma.position,
            radius * 0.5,
            Color::new(glow_inner.r, glow_inner.g, glow_inner.b, (outer_alpha as f32 * 0.9) as u8),
        );
    }

    let col = if plasma.state == PlasmaState::Flying {
        flying_col(plasma.timer)
    } else {
        plasma.state.col()
    };
    let src = source_rec(col, plasma.variant);
    let size = PLASMA_TEXTURE_SIZE * PLASMA_SCALE;
    let dest = Rectangle::new(plasma.position.x, plasma.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, src, dest, origin, plasma.rotation, Color::WHITE);
}

/// Draw this bolt's drop shadow - same tint/offset convention as
/// `draw_shell_shadow`, no glow halo (a shadow is a flat silhouette, not a
/// light source) - still reads from `plasma.variant`'s own row since the two
/// rows aren't pixel-identical (unlike shells.png's Flying column), just
/// drawn as a flat black silhouette regardless of colour, same as every
/// other shadow pass in the game. Caller (`Game::render`) only calls this
/// while `plasma.state == PlasmaState::Flying`.
pub fn draw_plasma_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, plasma: &Plasma) {
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
