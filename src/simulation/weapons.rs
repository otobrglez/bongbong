//! Firing. Spawning shells, plasma bolts, bullets, seeker missiles and
//! laser beams from a tank's muzzle (with recoil), ticking a twin-barrel
//! chassis's queued second shot, a minigun burst and a missile volley, the
//! per-weapon trigger dispatch the
//! player and every enemy share, and the `Projectile` view of the three
//! projectile types that lets one hit-resolution loop serve them all.

use crate::tuning::tuning;
use hecs::Entity;
use rand::RngExt;
use crate::math::Vec2;

use crate::bullet::{Bullet, BulletState};
use crate::laser::LaserVariant;
use crate::missile::Missile;
use crate::physics::Physics;
use crate::plasma::{Plasma, PlasmaState};
use crate::shell::{Owner, Shell, ShellState};
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, MinigunBurst, MissileVolley, PendingPlasmaShot, PendingShot, Tank};
use crate::{
    MISSILE_TUBE_OFFSETS,
    Position,
};

use super::Event;
use super::hits::{obstacle_reflect_axis, TerrainBox};
use super::Frame;

/// How far a laser beam's hit segment reaches past its muzzle on every
/// field whose diagonal it outruns - every arena and most fields - so it
/// always meets a wall before running out of room. A larger field's beam
/// reaches across it (`laser_reach`).
const LASER_MAX_RANGE: f32 = 4000.0;

/// How far a laser beam is traced on a field `field` px across: past the
/// field's diagonal by two cells, never shorter than `LASER_MAX_RANGE`.
/// The room's beam (`resolve_lasers`) and a client's drawn one
/// (`net::round`) read it alike.
pub fn laser_reach(field: (f32, f32)) -> f32 {
    LASER_MAX_RANGE.max(field.0.hypot(field.1) + 2.0 * crate::OBSTACLE_GRID_SIZE)
}

/// `shot`'s segment reaching `reach` px past its muzzle: the shot as built
/// when that is no further than `LASER_MAX_RANGE`, so every field the
/// fixed range already spans traces the very same segment.
pub(super) fn laser_end(shot: &PendingLaserShot, reach: f32) -> Position {
    if reach <= LASER_MAX_RANGE {
        return shot.end;
    }
    let k = reach / LASER_MAX_RANGE;
    Position::new(shot.start.x + (shot.end.x - shot.start.x) * k, shot.start.y + (shot.end.y - shot.start.y) * k)
}

/// Half-width of a laser beam's hit segment - a shell's, so a beam lands on
/// the same boxes a shell would.
pub(super) fn laser_beam_half_width() -> f32 {
    tuning().shell_hit_half_extent
}

/// A laser shot queued during the tank loops and resolved once they are
/// done (no other mutable tank query may run while they iterate).
pub(super) struct PendingLaserShot {
    /// Where the beam is judged from: the gun line's muzzle
    /// (`Tank::gun_line_muzzle`).
    pub start: Position,
    /// Far end of the un-clipped beam; the hit test finds where along
    /// `start..end` it actually stops.
    pub end: Position,
    /// Where the beam is drawn from: the laser module's lens beside the gun
    /// (`tank_art::LASER_MUZZLE`), to where the gun line stopped.
    pub lens: Position,
    pub shooter_row: i32,
    pub owner: Owner,
    pub variant: LaserVariant,
}

/// One frame of the flamethrower's stream (docs/flamethrower-prd.md):
/// the muzzle, the unit facing and the nominal reach. `Game::resolve_flames`
/// caps the reach at the first solid tile and applies the cone.
///
/// The cone is judged from the gun line's muzzle (`origin`); the jet is
/// drawn from the flamethrower module's nozzle beside the gun (`nozzle`)
/// to the end of that reach - `drawn`, which everything that paints or
/// lights the jet reads.
#[derive(Clone, Copy, Debug)]
pub struct FlameJet {
    pub origin: Position,
    /// Where the jet is drawn from: the flamethrower module's nozzle
    /// (`tank_art::FLAME_MUZZLE`).
    pub nozzle: Position,
    pub dir: Vec2,
    pub range: f32,
    /// Effective reach after `resolve_flames` capped it at the first
    /// solid tile; equal to `range` until then.
    pub reach: f32,
    pub owner: Owner,
    pub shooter: Entity,
}

impl FlameJet {
    /// The jet as it is drawn: from the nozzle to the end of the cone's
    /// reach on the gun line, as (start, unit direction, length).
    pub fn drawn(&self) -> (Position, Vec2, f32) {
        let end = self.origin + self.dir * self.reach;
        let span = end - self.nozzle;
        let length = span.length();
        if length <= f32::EPSILON {
            return (self.nozzle, self.dir, 0.0);
        }
        (self.nozzle, span / length, length)
    }
}

pub(super) fn flame_jet(tank: &Tank, owner: Owner, shooter: Entity) -> FlameJet {
    let rot = tank.rotation.to_radians();
    let dir = Vec2::new(rot.sin(), -rot.cos());
    let origin = tank.gun_line_muzzle(dir);
    let nozzle = tank.turret_point(crate::tank_art::FLAME_MUZZLE[tank.row as usize]);
    let range = tuning().flame_range;
    FlameJet { origin, nozzle, dir, range, reach: range, owner, shooter }
}

/// Damage range of one laser shot: LASER_DAMAGE_MIN..MAX scaled by the
/// shooter's chassis class and the beam variant.
pub(super) fn laser_damage_range(shot: &PendingLaserShot) -> (f32, f32) {
    let factor = tuning().tank_damage_factor[shot.shooter_row as usize] * shot.variant.damage_factor();
    (tuning().laser_damage_min * factor, tuning().laser_damage_max * factor)
}

/// Build a laser shot along `tank`'s facing: judged along the gun line from
/// its muzzle and drawn from the laser module's lens beside it. `aim_offset`
/// (degrees) is the same point-blank misfire skew a shell takes; 0.0 for a
/// clean shot. One beam per trigger pull regardless of chassis.
fn laser_shot(tank: &Tank, owner: Owner, aim_offset: f32, variant: LaserVariant) -> PendingLaserShot {
    let rot = (tank.rotation + aim_offset).to_radians();
    let dir = Vec2::new(rot.sin(), -rot.cos());
    let start = tank.gun_line_muzzle(dir);
    let end = Position::new(start.x + dir.x * LASER_MAX_RANGE, start.y + dir.y * LASER_MAX_RANGE);
    PendingLaserShot {
        start,
        end,
        lens: tank.turret_point(crate::tank_art::LASER_MUZZLE[tank.row as usize]),
        shooter_row: tank.row,
        owner,
        variant,
    }
}

/// Firing recoil: push the shooter back along the shot's own travel axis
/// (so a misfire's skew kicks the same way it skews the shot) at `speed`
/// px/s, normalized against the chassis-free baseline mass and capped at
/// `max_speed`. Returns the velocity change, `None` when nothing was
/// pushed.
///
/// Only a missile launch's goes on to `Frame::shoves`. A client that owns
/// its hull kicks it itself at every shell, bolt and bullet it launches
/// (`net::predict`, `Game::seat_recoil`), so an `Event::Shoved` for those
/// would kick it twice; it draws no missile, so that kick is the room's to
/// tell it about.
fn apply_recoil(physics: &mut Physics, tank: &Tank, velocity: Vec2, speed: f32, max_speed: f32) -> Option<Vec2> {
    let len = (velocity.x * velocity.x + velocity.y * velocity.y).sqrt();
    let handle = tank.body?;
    if len <= f32::EPSILON {
        return None;
    }
    let reference_mass = tank.scale * tank.scale;
    let push = (speed * reference_mass / tank.mass()).min(max_speed);
    let impulse = push * tank.mass() / len;
    let kick = Position::new(-velocity.x * impulse, -velocity.y * impulse);
    physics.apply_impulse(handle, kick);
    Some(Vec2::new(kick.x / tank.mass(), kick.y / tank.mass()))
}

/// Spawn one shell from `tank`: rolled drop shadow, muzzle-flash ripple,
/// recoil, queued into `f.pending_shells`. `lateral_offset` is zero for a
/// single-barrel shot, +/- the barrel offset for one half of a twin volley.
fn fire_shell(physics: &mut Physics, f: &mut Frame, tank: &Tank, owner: Owner, aim_offset: f32, lateral_offset: f32) {
    let mut shell = Shell::spawn(tank, owner, aim_offset, lateral_offset);
    shell.shadow_offset = f.rng.random_range(tuning().shell_shadow_offset_min..tuning().shell_shadow_offset_max);
    f.muzzle_flashes.push(Shockwave::new(shell.position));
    apply_recoil(physics, tank, shell.velocity, tuning().shell_recoil_speed, tuning().shell_recoil_max_speed);
    f.pending_shells.push(shell);
}

/// The plasma analogue of `fire_shell`, at PLASMA_* tuning.
fn fire_plasma(physics: &mut Physics, f: &mut Frame, tank: &Tank, owner: Owner, aim_offset: f32, lateral_offset: f32) {
    let mut plasma = Plasma::spawn(tank, owner, tank.plasma_variant, aim_offset, lateral_offset);
    plasma.shadow_offset = f.rng.random_range(tuning().plasma_shadow_offset_min..tuning().plasma_shadow_offset_max);
    f.muzzle_flashes.push(Shockwave::new(plasma.position));
    apply_recoil(physics, tank, plasma.velocity, tuning().plasma_recoil_speed, tuning().plasma_recoil_max_speed);
    f.pending_plasmas.push(plasma);
}

/// Fire one minigun bullet from `tank`'s minigun module (`Bullet::spawn`)
/// with a fresh `minigun_bullet_spread_deg` jitter on top of `aim_offset`.
/// No muzzle-flash ripple per bullet (a whole burst would stack into
/// mush) - the caller pushes one for the burst's first bullet; recoil is
/// per bullet at the minigun's much smaller kick. Returns the bullet's
/// spawn position.
fn fire_bullet(physics: &mut Physics, f: &mut Frame, tank: &Tank, owner: Owner, aim_offset: f32) -> Position {
    let spread = f.rng.random_range(-tuning().minigun_bullet_spread_deg..tuning().minigun_bullet_spread_deg);
    let mut bullet = Bullet::spawn(tank, owner, aim_offset + spread);
    bullet.shadow_offset = f.rng.random_range(tuning().minigun_bullet_shadow_offset_min..tuning().minigun_bullet_shadow_offset_max);
    apply_recoil(physics, tank, bullet.velocity, tuning().minigun_bullet_recoil_speed, tuning().minigun_bullet_recoil_max_speed);
    let position = bullet.position;
    f.pending_bullets.push(bullet);
    position
}

/// Launch one seeker missile from tube `tube` of `tank`'s launcher module:
/// from the tube's mouth (`tank_art::MISSILE_TUBES`), along the turret's
/// facing fanned `missile_fan_deg` per tube out from the centre, with the
/// launcher's aim point
/// (`missile_fallback_range` ahead, clamped to the field) as where it comes
/// down if the seek finds nothing. A puff at the tube, a small kick. No RNG.
fn fire_missile(physics: &mut Physics, f: &mut Frame, tank: &mut Tank, owner: Owner, aim_offset: f32, tube: u8) {
    let t = tuning();
    let offsets = MISSILE_TUBE_OFFSETS;
    let i = (tube as usize).min(offsets.len() - 1);
    let rot = (tank.rotation + aim_offset).to_radians();
    let dir = Vec2::new(rot.sin(), -rot.cos());
    let mouth = tank.turret_point(crate::tank_art::MISSILE_TUBES[tank.row as usize][i]);
    // Fan by where the tube sits: the middle pair a little, the outer pair
    // twice as much, each to its own side.
    let fan = t.missile_fan_deg * offsets[i] / 3.0;
    let fanned = (tank.rotation + aim_offset + fan).to_radians();
    let launch = Vec2::new(fanned.sin(), -fanned.cos());
    let ahead = tank.position + dir * t.missile_fallback_range;
    let aim = Position::new(ahead.x.clamp(0.0, f.width), ahead.y.clamp(0.0, f.height));
    f.muzzle_flashes.push(Shockwave::new(mouth));
    if let Some(dv) = apply_recoil(physics, tank, dir, t.missile_recoil_speed, t.missile_recoil_max_speed) {
        f.shoves.push(owner, dv);
    }
    tank.missile_tubes_empty = tank.missile_tubes_empty.saturating_add(1).min(offsets.len() as u8);
    f.pending_missiles.push(Missile::spawn(mouth, launch, owner, tube, aim));
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

    if let Some(mut volley) = tank.missile_volley {
        volley.timer -= dt;
        if volley.timer <= 0.0 {
            if wreck || tank.missile_ammo == 0 {
                tank.missile_volley = None;
            } else {
                tank.missile_ammo -= 1;
                if volley.next_tube == 0 {
                    // A new salvo: the pod has reloaded its tubes.
                    tank.missile_tubes_empty = 0;
                }
                fire_missile(physics, f, tank, owner, 0.0, volley.next_tube);
                volley.missiles_remaining -= 1;
                let tubes = tuning().missile_volley_size.clamp(1, MISSILE_TUBE_OFFSETS.len() as u32) as u8;
                volley.next_tube = (volley.next_tube + 1) % tubes;
                tank.missile_volley = if volley.missiles_remaining == 0 {
                    None
                } else {
                    // Back to the first tube means the salvo is spent: the
                    // longer gap while the pod reloads before the next.
                    volley.timer = if volley.next_tube == 0 {
                        tuning().missile_salvo_gap_seconds
                    } else {
                        tuning().missile_launch_delay_seconds
                    };
                    Some(volley)
                };
            }
        } else {
            tank.missile_volley = Some(volley);
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
    dispatch_fire_from(physics, f, tank, owner, aim_offset, None);
}

/// `dispatch_fire` for a tank whose entity is known, which the
/// flamethrower needs (its cone must skip the shooter). Every other
/// weapon ignores `shooter`.
pub(super) fn dispatch_fire_from(
    physics: &mut Physics,
    f: &mut Frame,
    tank: &mut Tank,
    owner: Owner,
    aim_offset: f32,
    shooter: Option<Entity>,
) {
    let lateral = tuning().tank_barrel_lateral_offset[tank.row as usize];
    let ammo_cost = if lateral > 0.0 { 2 } else { 1 };
    match tank.active_weapon() {
        ActiveWeapon::Flamethrower => {
            // Continuous, not paced: one jet per held frame, `dt` of fuel
            // each. `Fired` is recorded once per hold, not per frame.
            let Some(shooter) = shooter else { return };
            if tank.flame_fuel <= 0.0 {
                return;
            }
            if !tank.flame_held {
                tank.flame_held = true;
                f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: ActiveWeapon::Flamethrower.name() });
            }
            tank.flame_fuel = (tank.flame_fuel - f.dt).max(0.0);
            if tank.flame_fuel <= 0.0 {
                // Dry: the next press starts a new hold.
                tank.flame_held = false;
            }
            f.flame_jets.push(flame_jet(tank, owner, shooter));
        }
        ActiveWeapon::Laser => {
            tank.laser_charges -= 1;
            tank.fire_cooldown = tuning().player_fire_interval;
            f.pending_lasers.push(laser_shot(tank, owner, aim_offset, tank.laser_variant));
            f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: ActiveWeapon::Laser.name() });
            tank.kick_laser();
        }
        ActiveWeapon::Minigun => {
            if tank.minigun_ammo > 0 {
                f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: ActiveWeapon::Minigun.name() });
                tank.minigun_ammo -= 1;
                let muzzle = fire_bullet(physics, f, tank, owner, aim_offset);
                f.muzzle_flashes.push(Shockwave::new(muzzle));
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
        ActiveWeapon::Missiles => {
            // A volley of `missile_salvos` salvos, one missile per tube
            // each: the first tube now, the rest through
            // `tick_queued_shots`. Aimed dead ahead whatever the misfire
            // skew - the seek does the aiming.
            if tank.missile_ammo > 0 {
                f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: ActiveWeapon::Missiles.name() });
                tank.missile_ammo -= 1;
                tank.missile_tubes_empty = 0;
                fire_missile(physics, f, tank, owner, 0.0, 0);
                let volley = tuning().missile_volley_count();
                if volley > 1 {
                    let tubes = tuning().missile_volley_size.clamp(1, MISSILE_TUBE_OFFSETS.len() as u32) as u8;
                    let (next_tube, timer) = if tubes > 1 {
                        (1, tuning().missile_launch_delay_seconds)
                    } else {
                        (0, tuning().missile_salvo_gap_seconds)
                    };
                    tank.missile_volley = Some(MissileVolley { missiles_remaining: volley - 1, timer, next_tube });
                }
                tank.fire_cooldown = tuning().missile_volley_cooldown_seconds();
            }
        }
        ActiveWeapon::Plasma => {
            if tank.plasma_ammo >= ammo_cost {
                f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: ActiveWeapon::Plasma.name() });
                tank.plasma_ammo -= ammo_cost;
                tank.fire_cooldown = tuning().player_fire_interval;
                fire_plasma(physics, f, tank, owner, aim_offset, -lateral);
                tank.kick(true);
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
                f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: ActiveWeapon::Shell.name() });
                tank.shells_ammo -= ammo_cost;
                tank.fire_cooldown = tuning().player_fire_interval;
                fire_shell(physics, f, tank, owner, aim_offset, -lateral);
                tank.kick(false);
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
    fn velocity(&self) -> Vec2;
    /// The facing in degrees (0 = up), which a bounce turns.
    fn heading(&self) -> f32;
    fn owner(&self) -> Owner;
    /// The round's projectile number (`Shell::id`), given once by
    /// `Game::spawn_pending`.
    fn set_id(&mut self, id: u32);
    fn advance(&mut self, dt: f32);
    fn detonate(&mut self);
    fn hit_half_extent() -> f32;
    fn damage_range(&self) -> (f32, f32);
    /// Shove a surviving tank along the travel direction at this speed.
    fn knockback_speed() -> Option<f32>;
    /// Whether a surviving frog tries to hop away from this hit.
    fn frog_hops() -> bool;
    /// Bounce off `hit` instead of detonating, if this projectile can by
    /// its own rules (shells off Iron). A barrel's chance deflection goes
    /// through `can_bounce`/`reflect_off` instead.
    fn try_ricochet(&mut self, _hit: &TerrainBox) -> bool {
        false
    }
    /// Whether this kind can ricochet off a barrel at all - plasma never
    /// bounces, it is a bolt of energy.
    fn can_bounce() -> bool;
    /// Reflect off the face of `hit` that was struck and rewind to the
    /// pre-motion position so next frame starts clear of the tile - the
    /// geometry of a ricochet, with no gate of its own.
    fn reflect_off(&mut self, hit: &TerrainBox);
    /// Obstacle tiles this projectile rolled a pass-over on - the sweep
    /// skips them for the rest of its flight.
    fn passed_over(&self) -> &[Entity];
    fn note_passed_over(&mut self, entity: Entity);
    /// Bounce off a shielded tank centred at `center`: leave along
    /// `deflected_velocity`, rewind to the pre-motion position so next
    /// frame starts clear of the tank, and become `new_owner`'s projectile
    /// (the shielded tank's) so it can hit whoever fired it. Every
    /// projectile kind does this - unlike `try_ricochet`, which only
    /// shells get.
    fn deflect(&mut self, center: Position, new_owner: Owner);
}

/// How many ticks into the past stand the enemies and frogs a seat's
/// shell, bolt or bullet is swept against: its seat's rewind, taken at
/// spawn (`Game::spawn_pending`, `Game::seat_rewind`; lag compensation,
/// docs/online-coop-prd.md §4.16). A component beside the projectile
/// rather than a field of it, attached only when it is not zero - so an
/// enemy's shot and every shot of a local round spawn exactly as they
/// would without lag compensation - and zeroed when a shield turns the
/// shot back, since it is the shield's from then on, not the seat's that
/// aimed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Rewind(pub u8);

/// Velocity of a projectile bouncing off a shield centred at `center`: the
/// approach velocity mirrored across the radial normal from the centre to
/// where the projectile was before this frame's motion, so a shot straight
/// at the centre goes straight back and a glancing one skims off. Should
/// the mirror still point inward (a projectile that started inside the
/// box), it leaves straight outward along the normal instead.
pub(super) fn deflected_velocity(prev: Position, vel: Vec2, center: Position) -> Vec2 {
    let (nx, ny) = (prev.x - center.x, prev.y - center.y);
    let len = (nx * nx + ny * ny).sqrt();
    if len < f32::EPSILON {
        return Vec2::new(-vel.x, -vel.y);
    }
    let (nx, ny) = (nx / len, ny / len);
    let dot = vel.x * nx + vel.y * ny;
    let (rx, ry) = (vel.x - 2.0 * dot * nx, vel.y - 2.0 * dot * ny);
    if rx * nx + ry * ny > 0.0 {
        Vec2::new(rx, ry)
    } else {
        let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
        Vec2::new(nx * speed, ny * speed)
    }
}

/// The `Projectile::deflect` body shared by every projectile kind - they
/// all carry the same `velocity`/`rotation`/`position`/`prev_position`/
/// `owner` fields.
macro_rules! deflect_impl {
    () => {
        fn deflect(&mut self, center: Position, new_owner: Owner) {
            self.velocity = deflected_velocity(self.prev_position, self.velocity, center);
            self.rotation = self.velocity.x.atan2(-self.velocity.y).to_degrees();
            self.position = self.prev_position;
            self.owner = new_owner;
        }
        fn reflect_off(&mut self, hit: &TerrainBox) {
            let (reflect_x, reflect_y) = obstacle_reflect_axis(self.prev_position, hit);
            if reflect_x {
                self.velocity.x = -self.velocity.x;
            }
            if reflect_y {
                self.velocity.y = -self.velocity.y;
            }
            self.rotation = self.velocity.x.atan2(-self.velocity.y).to_degrees();
            self.position = self.prev_position;
        }
        fn passed_over(&self) -> &[Entity] {
            &self.passed_over
        }
        fn note_passed_over(&mut self, entity: Entity) {
            self.passed_over.push(entity);
        }
    };
}

fn side_damage(owner: Owner, shooter_row: i32) -> (f32, f32) {
    let (min, max) = match owner {
        Owner::Player(_) => (tuning().player_damage_min, tuning().player_damage_max),
        Owner::Enemy(_) => (tuning().enemy_damage_min, tuning().enemy_damage_max),
        // A tower has no chassis: its bullets carry the gun tower's damage.
        Owner::Tower { .. } => return (tuning().gun_tower_damage_min, tuning().gun_tower_damage_max),
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
    fn velocity(&self) -> Vec2 { self.velocity }
    fn heading(&self) -> f32 { self.rotation }
    fn owner(&self) -> Owner { self.owner }
    fn set_id(&mut self, id: u32) { self.id = id; }
    fn advance(&mut self, dt: f32) { self.update(dt); }
    fn detonate(&mut self) { Shell::detonate(self); }
    fn hit_half_extent() -> f32 { tuning().shell_hit_half_extent }
    fn damage_range(&self) -> (f32, f32) { side_damage(self.owner, self.shooter_row) }
    fn knockback_speed() -> Option<f32> { Some(tuning().shell_impact_knockback_speed) }
    fn frog_hops() -> bool { true }
    deflect_impl!();

    /// Shells ricochet off indestructible Iron while `bounces_left` lasts:
    /// reflect on the face that was struck and rewind to the pre-motion
    /// position so next frame starts clear of the tile.
    fn try_ricochet(&mut self, hit: &TerrainBox) -> bool {
        if self.bounces_left == 0 || !hit.material.is_permanent() {
            return false;
        }
        self.bounces_left -= 1;
        self.reflect_off(hit);
        true
    }
    fn can_bounce() -> bool { true }
}

impl Projectile for Bullet {
    fn is_flying(&self) -> bool { self.state == BulletState::Flying }
    fn is_done(&self) -> bool { self.done }
    fn position(&self) -> Position { self.position }
    fn set_position(&mut self, p: Position) { self.position = p; }
    fn prev_position(&self) -> Position { self.prev_position }
    fn begin_frame(&mut self) { self.prev_position = self.position; }
    fn velocity(&self) -> Vec2 { self.velocity }
    fn heading(&self) -> f32 { self.rotation }
    fn owner(&self) -> Owner { self.owner }
    fn set_id(&mut self, id: u32) { self.id = id; }
    fn advance(&mut self, dt: f32) { self.update(dt); }
    fn detonate(&mut self) { Bullet::detonate(self); }
    fn hit_half_extent() -> f32 { tuning().minigun_bullet_hit_half_extent }
    /// One shared range for player and enemy, chassis-scaled only; a
    /// tower's bullets carry the gun tower's own range.
    fn damage_range(&self) -> (f32, f32) {
        if self.owner.is_tower() {
            return side_damage(self.owner, self.shooter_row);
        }
        let factor = tuning().tank_damage_factor[self.shooter_row as usize];
        (tuning().minigun_bullet_damage_min * factor, tuning().minigun_bullet_damage_max * factor)
    }
    /// No per-bullet shove: a burst of them would read as juddering.
    fn knockback_speed() -> Option<f32> { None }
    /// No hop per bullet: several rounds in a third of a second would make
    /// the frog flail rather than dodge.
    fn frog_hops() -> bool { false }
    fn can_bounce() -> bool { true }
    deflect_impl!();
}

impl Projectile for Plasma {
    fn is_flying(&self) -> bool { self.state == PlasmaState::Flying }
    fn is_done(&self) -> bool { self.done }
    fn position(&self) -> Position { self.position }
    fn set_position(&mut self, p: Position) { self.position = p; }
    fn prev_position(&self) -> Position { self.prev_position }
    fn begin_frame(&mut self) { self.prev_position = self.position; }
    fn velocity(&self) -> Vec2 { self.velocity }
    fn heading(&self) -> f32 { self.rotation }
    fn owner(&self) -> Owner { self.owner }
    fn set_id(&mut self, id: u32) { self.id = id; }
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
    /// A bolt never ricochets (see docs/PLASMA_SPEC.md).
    fn can_bounce() -> bool { false }
    deflect_impl!();
}

#[cfg(test)]
mod deflect_tests {
    use super::*;

    fn outward(prev: Position, vel: Vec2, center: Position) -> f32 {
        let v = deflected_velocity(prev, vel, center);
        (v.x * (prev.x - center.x) + v.y * (prev.y - center.y)) / ((prev.x - center.x).hypot(prev.y - center.y))
    }

    #[test]
    fn a_head_on_shot_comes_straight_back_at_full_speed() {
        let v = deflected_velocity(Position::new(0.0, -100.0), Vec2::new(0.0, 300.0), Position::new(0.0, 0.0));
        assert!((v.x).abs() < 1e-3 && (v.y + 300.0).abs() < 1e-3, "{v:?}");
    }

    #[test]
    fn a_glancing_shot_skims_off_away_from_the_centre() {
        let prev = Position::new(-80.0, -60.0);
        let vel = Vec2::new(300.0, 0.0);
        let v = deflected_velocity(prev, vel, Position::new(0.0, 0.0));
        assert!(outward(prev, vel, Position::new(0.0, 0.0)) > 0.0, "leaves outward: {v:?}");
        assert!((v.x.hypot(v.y) - 300.0).abs() < 1e-3, "speed is preserved");
    }

    #[test]
    fn a_projectile_already_inside_leaves_straight_outward() {
        // Moving away from the centre already: the mirror would point back
        // in, so it exits along the normal instead.
        let prev = Position::new(10.0, 0.0);
        let v = deflected_velocity(prev, Vec2::new(200.0, 0.0), Position::new(0.0, 0.0));
        assert!(v.x > 0.0 && v.y.abs() < 1e-3, "{v:?}");
    }
}

#[cfg(test)]
mod laser_reach_tests {
    use super::*;

    fn shot(start: Position, dir: Vec2) -> PendingLaserShot {
        PendingLaserShot {
            start,
            end: Position::new(start.x + dir.x * LASER_MAX_RANGE, start.y + dir.y * LASER_MAX_RANGE),
            lens: start,
            shooter_row: 0,
            owner: Owner::Player(0),
            variant: LaserVariant::Red,
        }
    }

    #[test]
    fn every_field_the_fixed_range_spans_traces_the_same_segment() {
        // The largest shipped field and the 96 x 54 study map are well
        // inside it: their beams are the very segment they always were.
        for field in [(1088.0, 544.0), (1536.0, 864.0), (3072.0, 1728.0)] {
            assert_eq!(laser_reach(field), LASER_MAX_RANGE, "{field:?}");
        }
        let s = shot(Position::new(123.4, 567.8), Vec2::new(0.6, -0.8));
        assert_eq!(laser_end(&s, LASER_MAX_RANGE), s.end);
    }

    #[test]
    fn a_larger_field_is_crossed_corner_to_corner() {
        let field = (250.0 * crate::OBSTACLE_GRID_SIZE, 250.0 * crate::OBSTACLE_GRID_SIZE);
        let reach = laser_reach(field);
        assert!(reach > field.0.hypot(field.1), "{reach}");
        let s = shot(Position::new(0.0, 0.0), Vec2::new(field.0, field.1) * (1.0 / field.0.hypot(field.1)));
        let end = laser_end(&s, reach);
        assert!(end.x > field.0 && end.y > field.1, "{end:?} stops short of the far corner");
    }
}
