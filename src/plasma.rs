//! The plasma cannon's projectile (`Plasma`/`PlasmaState`): a limited-ammo
//! upgrade over a tank's traditional shell (see
//! `pickup::PickupKind::Plasma`, `Tank::plasma_ammo`), fired the exact same
//! way - straight down the barrel, a twin-barrel chassis firing one bolt per
//! barrel a beat apart (see `Tank::pending_plasma_shot`, mirroring
//! `shell::Shell`'s `PendingShot`) - but dealing PLASMA_DAMAGE_FACTOR more
//! damage and rendered as a glowing, pulsating orb (the runtime sine-wave
//! glow in `render::plasma::draw_plasma`, layered on top of a 4-frame baked
//! breathing animation while `Flying` - see `render::plasma::flying_col`/
//! docs/PLASMA_SPEC.md) that
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
use crate::math::Vec2;

use crate::shell::Owner;
use crate::tank::Tank;
use crate::Position;

/// A plasma bolt's lifecycle - same overall Fire/Flying/Hit shape as
/// `shell::ShellState`, just its own sheet (`static/plasma.png`) and its own
/// column layout (see docs/PLASMA_SPEC.md): `Flying` alone spans 4 columns
/// (see `render::plasma::flying_col`) rather than one, for a baked breathing animation
/// instead of a single static frame.
#[derive(Clone, Copy, PartialEq)]
pub enum PlasmaState {
    Fire0,  // col 0 - charge building at the muzzle
    Fire1,  // col 1 - bright flash as the bolt clears the barrel
    Fire2,  // col 2 - flash finishing, bolt pulling away
    Flying, // cols 3-6 (see `render::plasma::flying_col`) - glowing, breathing orb in the air
    Hit0,   // col 7 - impact burst starting
    Hit1,   // col 8 - electric burst expanding, arcs radiating outward
    Hit2,   // col 9 - burst dissipating
}

impl PlasmaState {

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

}

/// One plasma bolt - see this module's doc comment for how it compares to
/// `shell::Shell`/`bullet::Bullet`.
pub struct Plasma {
    pub state: PlasmaState,
    pub position: Position,
    /// Direction of travel while flying (pixels per second).
    pub velocity: Vec2,
    /// Facing angle in degrees (matches the tank's rotation when fired).
    pub rotation: f32,
    /// Time elapsed in the current state - kept growing (unbounded) while
    /// `Flying`, same trick `Shell`/`Bullet` already rely on (their own
    /// `Flying::duration()` being infinite means the increment-then-return
    /// in `update` never resets it) - `render::plasma::draw_plasma` reads this
    /// to phase both the runtime glow pulse and the baked `flying_col`
    /// breathing cycle.
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
    /// Obstacle tiles this projectile already rolled a pass-over on (a
    /// sandbag it sailed over) - skipped by every later hit sweep, since a
    /// segment ending inside a tile would otherwise re-roll it next frame.
    pub passed_over: Vec<hecs::Entity>,
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
        let dir = Vec2::new(rot.sin(), -rot.cos());
        let muzzle = tuning().tank_muzzle_forward_offset[tank.row as usize] * tank.scale;
        let hull_rot = tank.rotation.to_radians();
        let lateral = Vec2::new(hull_rot.cos(), hull_rot.sin()) * (lateral_offset * tank.scale);
        let position = Position::new(
            tank.position.x + dir.x * muzzle + lateral.x,
            tank.position.y + dir.y * muzzle + lateral.y,
        );
        Plasma {
            state: PlasmaState::Fire0,
            position,
            velocity: Vec2::new(dir.x * tuning().plasma_speed, dir.y * tuning().plasma_speed),
            rotation: tank.rotation + aim_offset,
            timer: 0.0,
            done: false,
            owner,
            variant,
            shooter_row: tank.row,
            shadow_offset: 0.0,
            prev_position: position,
            passed_over: Vec::new(),
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
