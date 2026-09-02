//! Damage and knockback: applying a resolved hit to whatever it struck,
//! ram contact between the player and an enemy, a wreck's explosion, and
//! the frog's evasive hop.

use rand::RngExt;
use rand::rngs::SmallRng;
use sola_raylib::core::math::Vector2;

use crate::ai::Ai;
use crate::frog::Frog;
use crate::obstacle::Obstacle;
use crate::physics::Physics;
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::{
    EXPLOSION_DAMAGE_MAX, EXPLOSION_DAMAGE_MIN, EXPLOSION_KNOCKBACK_SPEED, EXPLOSION_RADIUS,
    FROG_COLLIDER_HALF_EXTENT, FROG_HOP_ANGLE_FAN_DEG, FROG_HOP_ANGLE_JITTER_DEG,
    FROG_HOP_BOUNDS_MARGIN, KNOCKBACK_MAX_SPEED, KNOCKBACK_STRENGTH, MAX_DAMAGE, OBSTACLE_CLEAR,
    Position, RAM_DAMAGE_COOLDOWN, RAM_DAMAGE_MAX, RAM_DAMAGE_MIN,
};

use super::hits::ShellTarget;
use super::{with_frog_mut, Frame, Game};

/// What a hit does beyond damage: an optional shove (unit travel direction,
/// speed in px/s) for a surviving tank, and whether the frog tries to hop
/// away (along the given direction) when it survives.
pub(super) struct HitEffects {
    pub knockback: Option<(Vector2, f32)>,
    pub frog_hop: Option<Vector2>,
}

impl HitEffects {
    pub fn none() -> Self {
        HitEffects {
            knockback: None,
            frog_hop: None,
        }
    }
}

impl Game {
    /// Apply one resolved projectile/beam hit to `target`: roll damage in
    /// `dmg`, then per target kind - a tank is marked hit, either killed
    /// (recorded in `f.kills` for its shockwave/explosion) or shoved; an
    /// enemy that survives is told it was shot (`Ai::notify_hit`); the frog
    /// takes damage and either dies (shockwave) or hops away; an obstacle
    /// takes damage; a wall absorbs the shot. Wrecks and dead frogs ignore
    /// further hits.
    pub(super) fn apply_hit(&mut self, f: &mut Frame, target: ShellTarget, dmg: (f32, f32), effects: HitEffects) {
        let player = self.player.expect("player entity spawned in init");
        match target {
            ShellTarget::PlayerTank | ShellTarget::EnemyTank(_) => {
                let (entity, is_enemy) = match target {
                    ShellTarget::EnemyTank(e) => (e, true),
                    _ => (player, false),
                };
                let survived = {
                    let mut q = self.world.query_one::<&mut Tank>(entity);
                    let tank = q.get().expect("hit target always has a Tank");
                    if tank.is_wreck() {
                        false
                    } else {
                        let d = f.rng.random_range(dmg.0..dmg.1);
                        tank.damage = (tank.damage + d).min(MAX_DAMAGE);
                        tank.mark_hit();
                        if tank.is_wreck() {
                            f.kills.push((tank.position, is_enemy));
                            false
                        } else {
                            if let Some((dir, speed)) = effects.knockback {
                                knockback(tank, &mut self.physics, dir, speed);
                            }
                            true
                        }
                    }
                };
                if survived && is_enemy {
                    let mut q = self.world.query_one::<&mut Ai>(entity);
                    q.get().expect("enemy tanks always have an Ai").notify_hit();
                }
            }
            ShellTarget::Frog(entity) => {
                let (dead, pos, can_hop, hop_distance) = {
                    let mut q = self.world.query_one::<&mut Frog>(entity);
                    let frog = q.get().expect("hit target always has a Frog");
                    if frog.is_dead() {
                        (true, frog.position, false, 0.0)
                    } else {
                        frog.damage(f.rng.random_range(dmg.0..dmg.1));
                        (frog.is_dead(), frog.position, frog.can_hop(), frog.hop_distance())
                    }
                };
                if dead {
                    f.shock = Some(Shockwave { center: pos, time: 0.0 });
                } else if let (true, Some(away)) = (can_hop, effects.frog_hop) {
                    let obstacles = f.terrain.obstacle_centers();
                    let landing = frog_hop_target(&mut f.rng, pos, away, hop_distance, &obstacles, f.width, f.height);
                    if let Some(new_pos) = landing {
                        with_frog_mut(&self.world, entity, |fr| fr.start_hop(new_pos));
                    }
                }
            }
            ShellTarget::Obstacle(entity) => {
                let mut q = self.world.query_one::<&mut Obstacle>(entity);
                if let Ok(obstacle) = q.get() {
                    obstacle.damage(f.rng.random_range(dmg.0..dmg.1));
                }
            }
            ShellTarget::Wall => {}
        }
    }

    /// A tank's death: an outward shove that fades linearly to nothing at
    /// EXPLOSION_RADIUS, reaching every live tank regardless of side, plus
    /// a chip of damage only to the side opposing whoever died. Obstacles
    /// in range crack too (no side, no knockback). The frog is deliberately
    /// immune - it is a loss condition, so splash damage would be a real
    /// balance change rather than a detail. A chip that finishes off a
    /// tank pushes it onto `f.kills`, so it gets its own blast in turn.
    pub(super) fn apply_explosion(&mut self, f: &mut Frame, center: Position, victim_was_enemy: bool) {
        let player = self.player.expect("player entity spawned in init");
        {
            let mut q = self.world.query_one::<&mut Tank>(player);
            let tank = q.get().expect("player entity always has a Tank");
            explosion_hit(tank, center, victim_was_enemy, false, &mut self.physics, &mut f.rng, &mut f.kills);
        }
        for tank in self.world.query::<&mut Tank>().with::<&Ai>().iter() {
            explosion_hit(tank, center, !victim_was_enemy, true, &mut self.physics, &mut f.rng, &mut f.kills);
        }
        for obstacle in self.world.query::<&mut Obstacle>().iter() {
            explosion_hit_obstacle(obstacle, center, &mut f.rng);
        }
    }
}

/// Shove a live tank along `dir` (unit) at `speed` px/s - a real impulse
/// sized by the tank's own mass, so the velocity change is exact.
fn knockback(tank: &Tank, physics: &mut Physics, dir: Vector2, speed: f32) {
    let handle = tank.body.expect("tank should always have a physics body once spawned");
    physics.apply_impulse(handle, Position::new(dir.x * speed * tank.mass(), dir.y * speed * tank.mass()));
}

/// Ram contact between two live tanks on opposing sides: both take
/// RAM_DAMAGE_MIN..MAX once per RAM_DAMAGE_COOLDOWN (either tank still
/// cooling down blocks the whole exchange), and both are shoved apart along
/// the line between their centers, harder the faster they were closing,
/// split by mass so the lighter one flies further. A wreck on either side
/// means no exchange at all - a burning hulk neither deals nor takes ram
/// damage. Only ever called for player-vs-enemy contact: enemies bumping
/// each other are separated by the physics solver without damage. A tank
/// this kills is recorded in `kills`, tagged with its side.
pub(super) fn ram(
    a: &mut Tank,
    a_is_enemy: bool,
    b: &mut Tank,
    b_is_enemy: bool,
    physics: &mut Physics,
    rng: &mut SmallRng,
    kills: &mut Vec<(Position, bool)>,
) {
    if a.is_wreck() || b.is_wreck() || a.ram_cooldown > 0.0 || b.ram_cooldown > 0.0 {
        return;
    }
    let dmg = rng.random_range(RAM_DAMAGE_MIN..RAM_DAMAGE_MAX);
    a.damage = (a.damage + dmg).min(MAX_DAMAGE);
    b.damage = (b.damage + dmg).min(MAX_DAMAGE);
    a.mark_hit();
    b.mark_hit();
    a.ram_cooldown = RAM_DAMAGE_COOLDOWN;
    b.ram_cooldown = RAM_DAMAGE_COOLDOWN;
    if a.is_wreck() {
        kills.push((a.position, a_is_enemy));
    }
    if b.is_wreck() {
        kills.push((b.position, b_is_enemy));
    }

    let dx = a.position.x - b.position.x;
    let dy = a.position.y - b.position.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist <= 0.001 {
        return;
    }
    let axis = Vector2::new(dx / dist, dy / dist);
    let rel_x = a.velocity.x - b.velocity.x;
    let rel_y = a.velocity.y - b.velocity.y;
    let impact_speed = (rel_x * rel_x + rel_y * rel_y).sqrt();
    let push = (impact_speed * KNOCKBACK_STRENGTH).min(KNOCKBACK_MAX_SPEED);
    let total_mass = a.mass() + b.mass();
    // A tank this very hit just killed stays put, like any wreck.
    if !a.is_wreck() {
        let a_push = (push * 2.0 * b.mass() / total_mass).min(KNOCKBACK_MAX_SPEED);
        knockback(a, physics, axis, a_push);
    }
    if !b.is_wreck() {
        let b_push = (push * 2.0 * a.mass() / total_mass).min(KNOCKBACK_MAX_SPEED);
        knockback(b, physics, Vector2::new(-axis.x, -axis.y), b_push);
    }
}

/// One tank's share of a nearby explosion (see `Game::apply_explosion`):
/// a shove that fades linearly with distance and, only when `damage` is
/// true, a chip of damage scaled the same way. No-op on a wreck or a tank
/// outside EXPLOSION_RADIUS. `is_enemy` tags a resulting kill.
fn explosion_hit(
    tank: &mut Tank,
    center: Position,
    damage: bool,
    is_enemy: bool,
    physics: &mut Physics,
    rng: &mut SmallRng,
    kills: &mut Vec<(Position, bool)>,
) {
    if tank.is_wreck() {
        return;
    }
    let dx = tank.position.x - center.x;
    let dy = tank.position.y - center.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist > EXPLOSION_RADIUS {
        return;
    }
    let falloff = 1.0 - dist / EXPLOSION_RADIUS;

    if damage {
        let dmg = rng.random_range(EXPLOSION_DAMAGE_MIN..EXPLOSION_DAMAGE_MAX) * falloff;
        tank.damage = (tank.damage + dmg).min(MAX_DAMAGE);
        tank.mark_hit();
        if tank.is_wreck() {
            kills.push((tank.position, is_enemy));
            return;
        }
    }

    // The push is tuned against the chassis-free baseline mass (scale
    // squared), so a heavy chassis resists it and a light one flies.
    let reference_mass = tank.scale * tank.scale;
    let push = (EXPLOSION_KNOCKBACK_SPEED * falloff * reference_mass / tank.mass()).min(KNOCKBACK_MAX_SPEED);
    let axis = if dist > 0.001 {
        Vector2::new(dx / dist, dy / dist)
    } else {
        // Sitting exactly on the blast center: any direction beats none.
        Vector2::new(1.0, 0.0)
    };
    knockback(tank, physics, axis, push);
}

/// An obstacle's share of a nearby explosion: the same linear-falloff
/// damage as `explosion_hit`, no knockback (static body). A tile this
/// destroys is swept up by `Game::cleanup_done` like any other.
fn explosion_hit_obstacle(obstacle: &mut Obstacle, center: Position, rng: &mut SmallRng) {
    if obstacle.destroyed {
        return;
    }
    let dist = obstacle.position.distance_to(center);
    if dist > EXPLOSION_RADIUS {
        return;
    }
    let falloff = 1.0 - dist / EXPLOSION_RADIUS;
    obstacle.damage(rng.random_range(EXPLOSION_DAMAGE_MIN..EXPLOSION_DAMAGE_MAX) * falloff);
}

/// A landing spot for the frog's evasive hop, roughly `distance` px away
/// continuing along `away_dir` (only its angle matters): the ideal angle
/// plus a little jitter first, then FROG_HOP_ANGLE_FAN_DEG's offsets in
/// turn, taking the first candidate inside the battlefield
/// (FROG_HOP_BOUNDS_MARGIN) and clear of every obstacle. `None` when every
/// candidate is blocked - the hop is best-effort and the frog stays put.
pub(super) fn frog_hop_target(
    rng: &mut SmallRng,
    frog_pos: Position,
    away_dir: Vector2,
    distance: f32,
    obstacle_positions: &[Position],
    width: f32,
    height: f32,
) -> Option<Position> {
    let clear = FROG_COLLIDER_HALF_EXTENT.0.max(FROG_COLLIDER_HALF_EXTENT.1) + OBSTACLE_CLEAR;
    let jitter = rng
        .random_range(-FROG_HOP_ANGLE_JITTER_DEG..FROG_HOP_ANGLE_JITTER_DEG)
        .to_radians();
    let base_angle = away_dir.y.atan2(away_dir.x) + jitter;
    for offset_deg in FROG_HOP_ANGLE_FAN_DEG {
        let angle = base_angle + offset_deg.to_radians();
        let candidate = Position::new(
            frog_pos.x + angle.cos() * distance,
            frog_pos.y + angle.sin() * distance,
        );
        let in_bounds = candidate.x >= FROG_HOP_BOUNDS_MARGIN
            && candidate.x <= width - FROG_HOP_BOUNDS_MARGIN
            && candidate.y >= FROG_HOP_BOUNDS_MARGIN
            && candidate.y <= height - FROG_HOP_BOUNDS_MARGIN;
        let clear_of_obstacles = obstacle_positions.iter().all(|&p| candidate.distance_to(p) >= clear);
        if in_bounds && clear_of_obstacles {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod ram_tests {
    use super::*;
    use rand::SeedableRng;

    fn tank_at(x: f32, damage: f32, physics: &mut Physics) -> Tank {
        let mut tank = Tank {
            position: Position::new(x, 100.0),
            damage,
            ..Tank::default()
        };
        tank.body = Some(physics.spawn_tank(tank.position, tank.move_half_extents(false), tank.mass()));
        tank
    }

    #[test]
    fn a_wreck_neither_deals_nor_takes_ram_damage() {
        let mut physics = Physics::new();
        let mut wreck = tank_at(100.0, MAX_DAMAGE, &mut physics);
        let mut live = tank_at(130.0, 10.0, &mut physics);
        let mut rng = SmallRng::seed_from_u64(1);
        let mut kills = Vec::new();
        ram(&mut wreck, true, &mut live, false, &mut physics, &mut rng, &mut kills);
        assert_eq!(live.damage, 10.0);
        assert_eq!(live.ram_cooldown, 0.0);
        assert!(kills.is_empty());
    }

    #[test]
    fn live_tanks_exchange_damage_once_per_cooldown() {
        let mut physics = Physics::new();
        let mut a = tank_at(100.0, 0.0, &mut physics);
        let mut b = tank_at(130.0, 0.0, &mut physics);
        let mut rng = SmallRng::seed_from_u64(1);
        let mut kills = Vec::new();
        ram(&mut a, true, &mut b, false, &mut physics, &mut rng, &mut kills);
        assert!(a.damage >= RAM_DAMAGE_MIN && a.damage < RAM_DAMAGE_MAX);
        assert_eq!(a.damage, b.damage);
        assert_eq!(a.ram_cooldown, RAM_DAMAGE_COOLDOWN);
        let before = b.damage;
        ram(&mut a, true, &mut b, false, &mut physics, &mut rng, &mut kills);
        assert_eq!(b.damage, before, "second contact inside the cooldown must not re-damage");
    }
}
