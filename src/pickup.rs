//! Health and ammo pickups: small, inert collectibles that respawn near the
//! battlefield's corners (see `battlefield::sample_corner_position`,
//! `simulation::respawn_pickup`). Unlike `Obstacle`/`Frog` there's no
//! physics body - nothing should ever collide with a pickup, it's picked up
//! by touch alone (`Pickup::in_reach`, `simulation::pickup_phase`), so it's
//! just a position and a kind, checked against every living tank each frame.

use serde::{Deserialize, Serialize};
use crate::math::{Color, Rectangle, Vec2};

use crate::canvas::{Canvas, Sheet};
use crate::{PICKUP_SCALE, PICKUP_TEXTURE_SIZE, Position};

/// Which effect a pickup has when collected - see `simulation::collect_pickups`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PickupKind {
    Health,
    Ammo,
    /// Grants LASER_CHARGES_PER_PICKUP laser charges and queues the laser
    /// in the collector's FIFO weapon rotation (see
    /// `tank::Tank::weapon_queue` - the weapon currently firing keeps the
    /// trigger until depleted; a first pickup arms immediately) - while
    /// live and charged, firing resolves an instant beam hit instead of
    /// the tank's normal shell.
    Laser,
    /// Grants MINIGUN_AMMO_PER_PICKUP rounds of minigun ammo and queues
    /// the minigun (FIFO, as above) - while live and stocked, the trigger
    /// fires a multi-bullet burst instead of a normal shell - see
    /// `tank::Tank::active_weapon`.
    Minigun,
    /// Grants PLASMA_AMMO_PER_PICKUP rounds of plasma ammo and queues the
    /// plasma cannon (FIFO, as above) - while live and stocked, firing
    /// shoots a glowing plasma bolt from the barrel instead of a normal
    /// shell (one bolt per barrel on a twin-barrel chassis, same as
    /// `Shell`) - see `tank::Tank::active_weapon`.
    Plasma,
    /// Grants `missile_ammo_per_pickup` seeker missiles and queues the
    /// four-tube pod (FIFO, as above) - while live and stocked, a trigger
    /// pull fires a volley (one salvo of four by default) that climb,
    /// lock onto the nearest opposing tank and dive on it (`missile.rs`).
    /// Players and enemies both use it.
    Missiles,
    /// Sets `tank::Tank::speed_boost_timer` to SPEED_BOOST_DURATION_SECONDS -
    /// while positive, `Tank::effective_speed` is scaled by
    /// SPEED_BOOST_MULTIPLIER. A stat buff, not a weapon: picking up another
    /// one while already boosted refreshes the timer rather than stacking
    /// it, so a tank is only ever under one speed boost at a time.
    SpeedUp,
    /// Rainbow shield: heals the collector to full and fills
    /// `tank::Tank::shield_hp` to `shield_capacity`. Refills rather than
    /// stacks, like `SpeedUp`.
    ///
    /// The shield is a **pool of absorption on a short clock**: it soaks
    /// damage until spent, or until `shield_seconds` run out, and then
    /// shatters (`Event::ShieldBroken`), so concentrated fire ends it early and
    /// breaking contact is what preserves it (`Tank::tick_shield` refills a
    /// live shield after `shield_recharge_delay_seconds`; a shattered one
    /// never returns on its own). It is spent at two seams, because a
    /// projectile never reaches `Tank::take_damage`: that function is the
    /// absorb path for ram, blasts, flame, the frog and the laser, while
    /// shells, bullets and plasma bounce off in
    /// `Game::resolve_projectiles` and are charged
    /// `shield_deflect_cost_factor` times their own damage there.
    ///
    /// Usually an un-slotted bonus dropped next to a Health slot with
    /// `shield_near_health_chance` odds each time that slot is (re)spawned
    /// (`simulation::maybe_spawn_health_slot_bonuses`), though a map may
    /// also place one as a slot of its own.
    Shield,
    /// The flamethrower (docs/flamethrower-prd.md): grants
    /// `flame_fuel_per_pickup` seconds of fuel and queues the weapon (FIFO
    /// like the others). While live and fuelled, holding fire pours a
    /// short cone of flame that burns tanks, lights the ground and sets
    /// the map's own props alight. Player-only: an enemy driving over one
    /// leaves it where it is.
    Flamethrower,
    /// The frog health pack (docs/frog-health-pack-prd.md): heals the
    /// collector's *own* frog - `Game::frog` for a player,
    /// `Game::enemy_frog` for an enemy in a Hunt round - by
    /// `frog_pack_heal_fraction` of its max health, which is a full heal by
    /// default. The frog is otherwise the one thing in the game that only
    /// ever gets worse.
    ///
    /// One rule decides who collects it: **a tank collects a frog pack
    /// unless its own side's frog is alive and already at full health.** So
    /// a pack is left alone rather than wasted on a pristine frog, and a
    /// side with no frog at all (an enemy in Protect) consumes it for
    /// nothing, which is the denial pressure that makes it worth racing
    /// for.
    #[serde(rename = "frog_health")]
    FrogHealth,
    /// The tower pack (docs/defence-towers-prd.md section 9): restores
    /// every standing defence tower on the collector's side to full health
    /// and puts it out. Nothing on the tank itself.
    ///
    /// The frog pack's rule, for towers: **a tank collects a tower pack
    /// unless every standing tower on its side is already at full health
    /// and not burning.** A side with no standing tower takes it and wastes
    /// it. A map slot of its own, plus a bonus drop beside a Health slot
    /// while a player tower is hurt (`tower_pack_near_health_chance`).
    #[serde(rename = "tower_pack")]
    TowerPack,
    /// The heat shield (docs/volcano.md): red on top and black underneath,
    /// for `heat_shield_seconds` the collector takes no heat - not from
    /// lava, a burning bank, a burning cell, afterburn or a flamethrower's
    /// stream - so a lava stream is a road while it lasts. Blasts still
    /// hurt and shove. Refreshes rather than stacks, like `SpeedUp`.
    /// Player-only: an enemy driving over one leaves it where it is.
    #[serde(rename = "heat_shield")]
    HeatShield,
}

pub struct Pickup {
    pub kind: PickupKind,
    pub position: Position,
}

impl Pickup {
    /// Side length of this pickup's sprite on screen - drawn 1:1
    /// (PICKUP_SCALE), same "native res reads fine, no need to match the
    /// tanks' chunky look" reasoning as obstacles.
    pub fn size(&self) -> f32 {
        PICKUP_TEXTURE_SIZE * PICKUP_SCALE
    }

    /// Whether a hull box centred at `hull_center` with half-extents
    /// `hull_half`, grown by `pad` on every side, overlaps this pickup's
    /// square - the collection test (`simulation::pickup_phase`, `pad` is
    /// `pickup_collect_pad_px`). A box test rather than a radius from the
    /// centre, because the sprites touching is what a player sees: a disc
    /// that fits inside the touching rectangle collects from the hull's
    /// short side and falls short on its long one.
    pub fn in_reach(&self, hull_center: Position, hull_half: Position, pad: f32) -> bool {
        let half = self.size() * 0.5 + pad;
        (hull_center.x - self.position.x).abs() <= hull_half.x + half
            && (hull_center.y - self.position.y).abs() <= hull_half.y + half
    }
}

/// Draw one pickup, centered on its position, from its kind's own sheet
/// (`Sheet::Pickup` - each kind is a standalone 32x32 image rather than rows
/// in one shared sheet).
pub fn draw_pickup(c: &mut impl Canvas, pickup: &Pickup) {
    let size = pickup.size();
    let src = Rectangle::new(0.0, 0.0, PICKUP_TEXTURE_SIZE, PICKUP_TEXTURE_SIZE);
    let dest = Rectangle::new(pickup.position.x, pickup.position.y, size, size);
    let origin = Vec2::new(size / 2.0, size / 2.0);
    c.blit(Sheet::Pickup(pickup.kind), src, dest, origin, 0.0, Color::WHITE);
}

#[cfg(test)]
mod reach_tests {
    use super::*;

    fn pickup() -> Pickup {
        Pickup { kind: PickupKind::Health, position: Position::new(100.0, 100.0) }
    }

    /// An assault hull facing up (half-extents 16 x 22) against a 32 px
    /// pickup with a 6 px pad: the reach is 38 px beside, 44 px in front,
    /// and the corner takes both.
    #[test]
    fn touching_collects_and_a_pixel_short_does_not() {
        let p = pickup();
        let half = Position::new(16.0, 22.0);
        assert!(p.in_reach(Position::new(138.0, 100.0), half, 6.0));
        assert!(!p.in_reach(Position::new(139.0, 100.0), half, 6.0));
        assert!(p.in_reach(Position::new(100.0, 144.0), half, 6.0));
        assert!(!p.in_reach(Position::new(100.0, 145.0), half, 6.0));
        assert!(p.in_reach(Position::new(138.0, 144.0), half, 6.0));
        assert!(!p.in_reach(Position::new(139.0, 144.0), half, 6.0));
        // No pad is the sprites exactly touching; the facing swaps the
        // reach with the half-extents.
        assert!(p.in_reach(Position::new(132.0, 100.0), half, 0.0));
        assert!(!p.in_reach(Position::new(133.0, 100.0), half, 0.0));
        assert!(p.in_reach(Position::new(138.0, 100.0), Position::new(22.0, 16.0), 0.0));
    }
}
