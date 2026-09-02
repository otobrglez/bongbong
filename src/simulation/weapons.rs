//! Firing. Spawning shells, plasma bolts, bullets and laser beams from a
//! tank's muzzle (with recoil), ticking a twin-barrel chassis's queued
//! second shot and a minigun burst, the per-weapon trigger dispatch the
//! player and every enemy share, and the `Projectile` view of the three
//! projectile types that lets one hit-resolution loop serve them all.

use crate::tuning::tuning;
use rand::RngExt;
use sola_raylib::core::math::Vector2;

use crate::bullet::{Bullet, BulletState};
use crate::laser::LaserVariant;
use crate::obstacle::Material;
use crate::physics::Physics;
use crate::plasma::{Plasma, PlasmaState};
use crate::shell::{Owner, Shell, ShellState};
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, MinigunBurst, PendingPlasmaShot, PendingShot, Tank};
use crate::{
    Position,
};

use super::hits::{obstacle_reflect_axis, TerrainBox};
use super::Frame;

/// How far a laser beam's hit segment reaches past its muzzle - longer
/// than any battlefield diagonal, so it always meets a wall before running
/// out of room.
const LASER_MAX_RANGE: f32 = 4000.0;

/// Half-width of a laser beam's hit segment - a shell's, so a beam lands on
/// the same boxes a shell would.
pub(super) fn laser_beam_half_width() -> f32 {
    tuning().shell_hit_half_extent
}

/// A laser shot queued during the tank loops and resolved once they are
/// done (no other mutable tank query may run while they iterate).
pub(super) struct PendingLaserShot {
    pub start: Position,
    /// Far end of the un-clipped beam; the hit test finds where along
    /// `start..end` it actually stops.
    pub end: Position,
    pub shooter_row: i32,
    pub owner: Owner,
    pub variant: LaserVariant,
}

/// Damage range of one laser shot: LASER_DAMAGE_MIN..MAX scaled by the
/// shooter's chassis class and the beam variant.
pub(super) fn laser_damage_range(shot: &PendingLaserShot) -> (f32, f32) {
    let factor = tuning().tank_damage_factor[shot.shooter_row as usize] * shot.variant.damage_factor();
    (tuning().laser_damage_min * factor, tuning().laser_damage_max * factor)
}

/// Build a laser shot from `tank`'s muzzle along its facing. `aim_offset`
/// (degrees) is the same point-blank misfire skew a shell takes; 0.0 for a
/// clean shot. One centerline beam per trigger pull regardless of chassis.
fn laser_shot(tank: &Tank, owner: Owner, aim_offset: f32, variant: LaserVariant) -> PendingLaserShot {
    let rot = (tank.rotation + aim_offset).to_radians();
    let dir = Vector2::new(rot.sin(), -rot.cos());
    let muzzle = tuning().tank_muzzle_forward_offset[tank.row as usize] * tank.scale;
    let start = Position::new(tank.position.x + dir.x * muzzle, tank.position.y + dir.y * muzzle);
    let end = Position::new(start.x + dir.x * LASER_MAX_RANGE, start.y + dir.y * LASER_MAX_RANGE);
    PendingLaserShot {
        start,
        end,
        shooter_row: tank.row,
        owner,
        variant,
    }
}

/// Firing recoil: push the shooter back along the shot's own travel axis
/// (so a misfire's skew kicks the same way it skews the shot) at `speed`
/// px/s, normalized against the chassis-free baseline mass and capped at
/// `max_speed`.
fn apply_recoil(physics: &mut Physics, tank: &Tank, velocity: Vector2, speed: f32, max_speed: f32) {
    let len = (velocity.x * velocity.x + velocity.y * velocity.y).sqrt();
    let Some(handle) = tank.body else { return };
    if len <= f32::EPSILON {
        return;
    }
    let reference_mass = tank.scale * tank.scale;
    let push = (speed * reference_mass / tank.mass()).min(max_speed);
    let impulse = push * tank.mass() / len;
    physics.apply_impulse(handle, Position::new(-velocity.x * impulse, -velocity.y * impulse));
}

/// Spawn one shell from `tank`: rolled drop shadow, muzzle-flash ripple,
/// recoil, queued into `f.pending_shells`. `lateral_offset` is zero for a
/// single-barrel shot, +/- the barrel offset for one half of a twin volley.
fn fire_shell(physics: &mut Physics, f: &mut Frame, tank: &Tank, owner: Owner, aim_offset: f32, lateral_offset: f32) {
    let mut shell = Shell::spawn(tank, owner, aim_offset, lateral_offset);
    shell.shadow_offset = f.rng.random_range(tuning().shell_shadow_offset_min..tuning().shell_shadow_offset_max);
    f.muzzle_flashes.push(Shockwave { center: shell.position, time: 0.0 });
    apply_recoil(physics, tank, shell.velocity, tuning().shell_recoil_speed, tuning().shell_recoil_max_speed);
    f.pending_shells.push(shell);
}

/// The plasma analogue of `fire_shell`, at PLASMA_* tuning.
fn fire_plasma(physics: &mut Physics, f: &mut Frame, tank: &Tank, owner: Owner, aim_offset: f32, lateral_offset: f32) {
    let mut plasma = Plasma::spawn(tank, owner, tank.plasma_variant, aim_offset, lateral_offset);
    plasma.shadow_offset = f.rng.random_range(tuning().plasma_shadow_offset_min..tuning().plasma_shadow_offset_max);
    f.muzzle_flashes.push(Shockwave { center: plasma.position, time: 0.0 });
    apply_recoil(physics, tank, plasma.velocity, tuning().plasma_recoil_speed, tuning().plasma_recoil_max_speed);
    f.pending_plasmas.push(plasma);
}

/// Fire one minigun bullet dead-center from `tank`'s muzzle with a fresh
/// MINIGUN_BULLET_SPREAD_DEG jitter on top of `aim_offset`. No muzzle-flash
/// ripple per bullet (a whole burst would stack into mush) - the caller
/// pushes one for the burst's first bullet; recoil is per bullet at the
/// minigun's much smaller kick. Returns the bullet's spawn position.
fn fire_bullet(physics: &mut Physics, f: &mut Frame, tank: &Tank, owner: Owner, aim_offset: f32) -> Position {
    let spread = f.rng.random_range(-tuning().minigun_bullet_spread_deg..tuning().minigun_bullet_spread_deg);
    let mut bullet = Bullet::spawn(tank, owner, aim_offset + spread);
    bullet.shadow_offset = f.rng.random_range(tuning().minigun_bullet_shadow_offset_min..tuning().minigun_bullet_shadow_offset_max);
    apply_recoil(physics, tank, bullet.velocity, tuning().minigun_bullet_recoil_speed, tuning().minigun_bullet_recoil_max_speed);
    let position = bullet.position;
    f.pending_bullets.push(bullet);
    position
}

/// Tick a tank's queued shots: a twin-barrel chassis's second shell or
/// plasma bolt (`Tank::pending_shot`/`pending_plasma_shot`) and the rest of
/// a minigun burst (`Tank::minigun_burst`). Runs every frame whether or not
/// the trigger is still held, so a volley/burst always completes - unless
/// the tank is a wreck by then, which cancels it. Runs *before* this
/// frame's new fire input is handled.
pub(super) fn tick_queued_shots(physics: &mut Physics, f: &mut Frame, tank: &mut Tank, owner: Owner) {
    let dt = f.dt;
    let wreck = tank.is_wreck();

    if let Some(mut pending) = tank.pending_shot {
        pending.timer -= dt;
        if pending.timer <= 0.0 {
            if !wreck {
                fire_shell(physics, f, tank, owner, pending.aim_offset, pending.lateral_offset);
            }
            tank.pending_shot = None;
        } else {
            tank.pending_shot = Some(pending);
        }
    }

    if let Some(mut pending) = tank.pending_plasma_shot {
        pending.timer -= dt;
        if pending.timer <= 0.0 {
            if !wreck {
                fire_plasma(physics, f, tank, owner, pending.aim_offset, pending.lateral_offset);
            }
            tank.pending_plasma_shot = None;
        } else {
            tank.pending_plasma_shot = Some(pending);
        }
    }

    if let Some(mut burst) = tank.minigun_burst {
        burst.timer -= dt;
        if burst.timer <= 0.0 {
            if wreck || tank.minigun_ammo == 0 {
                // Destroyed or dry mid-burst: stop short, no fallback to shells.
                tank.minigun_burst = None;
            } else {
                tank.minigun_ammo -= 1;
                fire_bullet(physics, f, tank, owner, burst.aim_offset);
                burst.bullets_remaining -= 1;
                tank.minigun_burst = if burst.bullets_remaining == 0 {
                    None
                } else {
                    burst.timer = tuning().minigun_bullet_delay_seconds;
                    Some(burst)
                };
            }
        } else {
            tank.minigun_burst = Some(burst);
        }
    }
}

/// Fire `tank`'s active weapon once. The caller has already decided the
/// trigger is pulled and `fire_cooldown` has expired; this spends ammo,
/// sets the next cooldown and launches the shot - a laser beam, a minigun
/// burst (first bullet now, the rest via `tick_queued_shots`), or one
/// plasma bolt / shell per barrel: a twin-barrel chassis (nonzero
/// TANK_BARREL_LATERAL_OFFSET_BY_ROW) fires the left barrel now and queues
/// the right one TANK_TWIN_SHOT_DELAY_SECONDS later, costing 2 ammo. A
/// weapon short of the ammo it needs simply does nothing.
pub(super) fn dispatch_fire(physics: &mut Physics, f: &mut Frame, tank: &mut Tank, owner: Owner, aim_offset: f32) {
    let lateral = tuning().tank_barrel_lateral_offset[tank.row as usize];
    let ammo_cost = if lateral > 0.0 { 2 } else { 1 };
    match tank.active_weapon() {
        ActiveWeapon::Laser => {
            tank.laser_charges -= 1;
            tank.fire_cooldown = tuning().player_fire_interval;
            f.pending_lasers.push(laser_shot(tank, owner, aim_offset, tank.laser_variant));
        }
        ActiveWeapon::Minigun => {
            if tank.minigun_ammo > 0 {
                tank.minigun_ammo -= 1;
                let muzzle = fire_bullet(physics, f, tank, owner, aim_offset);
                f.muzzle_flashes.push(Shockwave { center: muzzle, time: 0.0 });
                if tuning().minigun_burst_size > 1 {
                    tank.minigun_burst = Some(MinigunBurst {
                        bullets_remaining: tuning().minigun_burst_size - 1,
                        timer: tuning().minigun_bullet_delay_seconds,
                        aim_offset,
                    });
                }
                tank.fire_cooldown = tuning().minigun_burst_cooldown_seconds();
            }
        }
        ActiveWeapon::Plasma => {
            if tank.plasma_ammo >= ammo_cost {
                tank.plasma_ammo -= ammo_cost;
                tank.fire_cooldown = tuning().player_fire_interval;
                fire_plasma(physics, f, tank, owner, aim_offset, -lateral);
                if lateral > 0.0 {
                    tank.pending_plasma_shot = Some(PendingPlasmaShot {
                        timer: tuning().tank_twin_shot_delay_seconds,
                        aim_offset,
                        lateral_offset: lateral,
                    });
                }
            }
        }
        ActiveWeapon::Shell => {
            if tank.shells_ammo >= ammo_cost {
                tank.shells_ammo -= ammo_cost;
                tank.fire_cooldown = tuning().player_fire_interval;
                fire_shell(physics, f, tank, owner, aim_offset, -lateral);
                if lateral > 0.0 {
                    tank.pending_shot = Some(PendingShot {
                        timer: tuning().tank_twin_shot_delay_seconds,
                        aim_offset,
                        lateral_offset: lateral,
                    });
                }
            }
        }
    }
}

/// The common view of a shell, bullet or plasma bolt that
/// `Game::resolve_projectiles` needs: where it is and was this frame, who
/// fired it, how it moves and detonates, and its weapon's tuning.
pub(super) trait Projectile: hecs::Component {
    fn is_flying(&self) -> bool;
    fn is_done(&self) -> bool;
    fn position(&self) -> Position;
    fn set_position(&mut self, p: Position);
    fn prev_position(&self) -> Position;
    /// Mark the current position as this frame's segment start.
    fn begin_frame(&mut self);
    fn velocity(&self) -> Vector2;
    fn owner(&self) -> Owner;
    fn advance(&mut self, dt: f32);
    fn detonate(&mut self);
    fn hit_half_extent() -> f32;
    fn damage_range(&self) -> (f32, f32);
    /// Shove a surviving tank along the travel direction at this speed.
    fn knockback_speed() -> Option<f32>;
    /// Whether a surviving frog tries to hop away from this hit.
    fn frog_hops() -> bool;
    /// Bounce off `hit` instead of detonating, if this projectile can.
    fn try_ricochet(&mut self, _hit: &TerrainBox) -> bool {
        false
    }
}

fn side_damage(owner: Owner, shooter_row: i32) -> (f32, f32) {
    let (min, max) = match owner {
        Owner::Player => (tuning().player_damage_min, tuning().player_damage_max),
        Owner::Enemy(_) => (tuning().enemy_damage_min, tuning().enemy_damage_max),
    };
    let factor = tuning().tank_damage_factor[shooter_row as usize];
    (min * factor, max * factor)
}

impl Projectile for Shell {
    fn is_flying(&self) -> bool { self.state == ShellState::Flying }
    fn is_done(&self) -> bool { self.done }
    fn position(&self) -> Position { self.position }
    fn set_position(&mut self, p: Position) { self.position = p; }
    fn prev_position(&self) -> Position { self.prev_position }
    fn begin_frame(&mut self) { self.prev_position = self.position; }
    fn velocity(&self) -> Vector2 { self.velocity }
    fn owner(&self) -> Owner { self.owner }
    fn advance(&mut self, dt: f32) { self.update(dt); }
    fn detonate(&mut self) { Shell::detonate(self); }
    fn hit_half_extent() -> f32 { tuning().shell_hit_half_extent }
    fn damage_range(&self) -> (f32, f32) { side_damage(self.owner, self.shooter_row) }
    fn knockback_speed() -> Option<f32> { Some(tuning().shell_impact_knockback_speed) }
    fn frog_hops() -> bool { true }

    /// Shells ricochet off indestructible Iron while `bounces_left` lasts:
    /// reflect on the face that was struck and rewind to the pre-motion
    /// position so next frame starts clear of the tile.
    fn try_ricochet(&mut self, hit: &TerrainBox) -> bool {
        if self.bounces_left == 0 || hit.material != Material::Iron {
            return false;
        }
        let (reflect_x, reflect_y) = obstacle_reflect_axis(self.prev_position, hit);
        self.bounces_left -= 1;
        if reflect_x {
            self.velocity.x = -self.velocity.x;
        }
        if reflect_y {
            self.velocity.y = -self.velocity.y;
        }
        self.rotation = self.velocity.x.atan2(-self.velocity.y).to_degrees();
        self.position = self.prev_position;
        true
    }
}

impl Projectile for Bullet {
    fn is_flying(&self) -> bool { self.state == BulletState::Flying }
    fn is_done(&self) -> bool { self.done }
    fn position(&self) -> Position { self.position }
    fn set_position(&mut self, p: Position) { self.position = p; }
    fn prev_position(&self) -> Position { self.prev_position }
    fn begin_frame(&mut self) { self.prev_position = self.position; }
    fn velocity(&self) -> Vector2 { self.velocity }
    fn owner(&self) -> Owner { self.owner }
    fn advance(&mut self, dt: f32) { self.update(dt); }
    fn detonate(&mut self) { Bullet::detonate(self); }
    fn hit_half_extent() -> f32 { tuning().minigun_bullet_hit_half_extent }
    /// One shared range for player and enemy, chassis-scaled only.
    fn damage_range(&self) -> (f32, f32) {
        let factor = tuning().tank_damage_factor[self.shooter_row as usize];
        (tuning().minigun_bullet_damage_min * factor, tuning().minigun_bullet_damage_max * factor)
    }
    /// No per-bullet shove: a burst of them would read as juddering.
    fn knockback_speed() -> Option<f32> { None }
    /// No hop per bullet: several rounds in a third of a second would make
    /// the frog flail rather than dodge.
    fn frog_hops() -> bool { false }
}

impl Projectile for Plasma {
    fn is_flying(&self) -> bool { self.state == PlasmaState::Flying }
    fn is_done(&self) -> bool { self.done }
    fn position(&self) -> Position { self.position }
    fn set_position(&mut self, p: Position) { self.position = p; }
    fn prev_position(&self) -> Position { self.prev_position }
    fn begin_frame(&mut self) { self.prev_position = self.position; }
    fn velocity(&self) -> Vector2 { self.velocity }
    fn owner(&self) -> Owner { self.owner }
    fn advance(&mut self, dt: f32) { self.update(dt); }
    fn detonate(&mut self) { Plasma::detonate(self); }
    fn hit_half_extent() -> f32 { tuning().plasma_hit_half_extent }
    fn damage_range(&self) -> (f32, f32) {
        let (min, max) = side_damage(self.owner, self.shooter_row);
        let factor = tuning().plasma_damage_factor * self.variant.damage_factor();
        (min * factor, max * factor)
    }
    fn knockback_speed() -> Option<f32> { Some(tuning().plasma_impact_knockback_speed) }
    fn frog_hops() -> bool { true }
}
