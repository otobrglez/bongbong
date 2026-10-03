//! Pickups: inert collectibles at the map's own slots, respawning after a
//! delay once taken (`simulation::pickup_phase`, `respawn_from_slots`).
//! Unlike `Obstacle`/`Frog` there's no physics body - nothing should ever
//! collide with a pickup, it's picked up by touch alone (`Pickup::in_reach`),
//! so it's a position and a kind, checked against every living tank each
//! frame.
//!
//! Each one is drawn as a supply crate with its symbol on the lid
//! (docs/CRATES_SPEC.md): the crate sheet's row for its kind, a drop shadow
//! like an obstacle's, the air drop it comes down in and the glint it idles
//! with (`crate_fx.rs`). The symbols' inks (`PickupKind::ink`) are the
//! sheets' own colours, loud on purpose so a crate is spotted at a glance.

use serde::{Deserialize, Serialize};
use crate::math::{Color, Rectangle, Vec2};

use crate::canvas::{Canvas, Sheet};
use crate::{CRATE_CELL, CRATE_COL_INTACT, PICKUP_GLYPH_CELL, PICKUP_SIZE, Position};

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
}

impl PickupKind {
    /// Every kind in declaration order: the crate and symbol sheets' row
    /// order (`row`).
    pub const ALL: [PickupKind; 11] = [
        PickupKind::Health,
        PickupKind::Ammo,
        PickupKind::Laser,
        PickupKind::Minigun,
        PickupKind::Plasma,
        PickupKind::Missiles,
        PickupKind::SpeedUp,
        PickupKind::Shield,
        PickupKind::Flamethrower,
        PickupKind::FrogHealth,
        PickupKind::TowerPack,
    ];

    /// This kind's row on static/crates_sheet.png and
    /// static/pickup_glyphs.png (tools/spritegen/gen_crates.py's `KINDS`).
    pub fn row(self) -> usize {
        match self {
            PickupKind::Health => 0,
            PickupKind::Ammo => 1,
            PickupKind::Laser => 2,
            PickupKind::Minigun => 3,
            PickupKind::Plasma => 4,
            PickupKind::Missiles => 5,
            PickupKind::SpeedUp => 6,
            PickupKind::Shield => 7,
            PickupKind::Flamethrower => 8,
            PickupKind::FrogHealth => 9,
            PickupKind::TowerPack => 10,
        }
    }

    /// The symbol's ink - shade, base, light - as the sheets paint it
    /// (`punypalette.PICKUP_INK`): what the crate's light, the opening's
    /// ring and the spilled symbol are coloured by. The bases are the HUD's
    /// weapon colours where the HUD has one.
    pub fn ink(self) -> [Color; 3] {
        let rgb = |c: u32| Color::new((c >> 16) as u8, (c >> 8) as u8, c as u8, 255);
        let [shade, base, light] = match self {
            PickupKind::Health => [0xA82B26, 0xE84A3C, 0xFF8A70],
            PickupKind::Ammo => [0xB48A22, 0xF2C84B, 0xFFE79A],
            PickupKind::Laser => [0xB82270, 0xFF4FA8, 0xFF9FD0],
            PickupKind::Minigun => [0x7F909B, 0xC9D6DE, 0xF1F6F8],
            PickupKind::Plasma => [0x1A9C93, 0x3FE0CC, 0xA8FFF3],
            PickupKind::Missiles => [0x6F9C1E, 0xB6E848, 0xE0FF9A],
            PickupKind::SpeedUp => [0xC79A1A, 0xFFD93D, 0xFFF2A8],
            PickupKind::Shield => [0x6B49C9, 0xA77BFF, 0xD3BCFF],
            PickupKind::Flamethrower => [0xC2501A, 0xFF8A2B, 0xFFC27A],
            PickupKind::FrogHealth => [0x3F9A35, 0x7EDB5A, 0xC2F7A0],
            PickupKind::TowerPack => [0x4F6FC7, 0x8FB0FF, 0xCFDCFF],
        };
        [rgb(shade), rgb(base), rgb(light)]
    }

    /// Whether a broken crate of this kind cooks off - ordnance and energy
    /// go up in a blast of their own - rather than spill its contents
    /// (`simulation::crates`).
    pub fn cooks_off(self) -> bool {
        match self {
            PickupKind::Ammo | PickupKind::Minigun | PickupKind::Missiles | PickupKind::Flamethrower => true,
            PickupKind::Laser | PickupKind::Plasma => true,
            PickupKind::Health | PickupKind::SpeedUp | PickupKind::Shield | PickupKind::FrogHealth | PickupKind::TowerPack => false,
        }
    }
}

pub struct Pickup {
    pub kind: PickupKind,
    pub position: Position,
    /// The round clock (`Game::time`) when it came down; `None` for a crate
    /// that was there when the round began. Only the drawing reads it - the
    /// air drop plays while the crate is younger than its landing
    /// (`crate_fx::drop`) - and a crate can be taken the frame it appears,
    /// mid-drop, so no rule waits on it and no replay moves.
    pub dropped_at: Option<f32>,
    /// The crate's hit points (`crate_hp` fresh), while crates are
    /// breakable (`simulation::crates`).
    pub health: f32,
    /// Seconds a burning crate has left before it falls in; `None` while
    /// it is not burning.
    pub burn: Option<f32>,
    /// Seconds a broken crate's contents have left lying loose on the
    /// cell; `None` for a crate. Loose contents are drawn as the bare
    /// symbol and taken like any pickup.
    pub loose: Option<f32>,
}

impl Pickup {
    /// A pickup that was on the field from the round's start.
    pub fn placed(kind: PickupKind, position: Position) -> Self {
        Pickup::dropped(kind, position, None)
    }

    /// A crate that came down at `dropped_at` on the round clock (`None`
    /// for one that stood there from the start), whole.
    pub fn dropped(kind: PickupKind, position: Position, dropped_at: Option<f32>) -> Self {
        Pickup { kind, position, dropped_at, health: crate::tuning::tuning().crate_hp, burn: None, loose: None }
    }

    /// Side length of this pickup's square: one cell, what a hull touches
    /// to collect it.
    pub fn size(&self) -> f32 {
        PICKUP_SIZE
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

/// The crate sheet's cell for `kind` in column `col`.
pub fn crate_src(kind: PickupKind, col: usize) -> Rectangle {
    Rectangle::new(col as f32 * CRATE_CELL, kind.row() as f32 * CRATE_CELL, CRATE_CELL, CRATE_CELL)
}

/// The symbol sheet's cell for `kind`.
pub fn glyph_src(kind: PickupKind) -> Rectangle {
    Rectangle::new(0.0, kind.row() as f32 * PICKUP_GLYPH_CELL, PICKUP_GLYPH_CELL, PICKUP_GLYPH_CELL)
}

/// Draw one pickup at `time` on the round clock: a broken crate's loose
/// contents as the bare symbol, blinking out at the end; a burning crate
/// charred with flames off its lid, a hurt one split; a crate in its air
/// drop while it is coming down (`crate_fx::drop`); else the crate standing
/// on its cell with a drop shadow like an obstacle's and the glint it idles
/// with (`crate_fx::glint_col`). `shadows` is `Game::shadows_enabled`.
pub fn draw_pickup(c: &mut impl Canvas, pickup: &Pickup, time: f32, shadows: bool) {
    let t = crate::tuning::tuning();
    let at = pickup.position;
    if let Some(left) = pickup.loose {
        // Spilled: the bare symbol on the ground, blinking out over its
        // last second and a half.
        if left < 1.5 && ((time / 0.1) as i32) % 2 == 0 {
            return;
        }
        if shadows {
            draw_glyph(c, pickup.kind, Position::new(at.x + 2.0, at.y + 4.0), PICKUP_GLYPH_CELL, Color::new(0, 0, 0, 77));
        }
        draw_glyph(c, pickup.kind, at, PICKUP_GLYPH_CELL, Color::WHITE);
        return;
    }
    if pickup.burn.is_some() || pickup.health < t.crate_hp * 0.5 {
        let col = if pickup.burn.is_some() { crate::CRATE_COL_CHARRED } else { crate::CRATE_COL_DAMAGED };
        draw_crate(c, pickup.kind, at, col, shadows, Color::WHITE);
        if pickup.burn.is_some() {
            crate::pyro::draw(c, &crate_flames(pickup, time));
        }
        return;
    }
    if let Some(drop) = pickup.dropped_at.and_then(|since| crate::crate_fx::drop(time - since, at, &t)) {
        if shadows && drop.shadow_alpha > 0.0 {
            let (w, h) = (drop.shadow_w, drop.shadow_h);
            let dest = Rectangle::new(
                (at.x - w * 0.5 + t.shadow_dir_x * t.obstacle_shadow_offset).round(),
                (at.y + CRATE_CELL * 0.5 - h + t.shadow_dir_y * t.obstacle_shadow_offset).round(),
                w,
                h,
            );
            let shadow = Color::new(0, 0, 0, (255.0 * t.obstacle_shadow_opacity * drop.shadow_alpha) as u8);
            c.blit(Sheet::Crates, crate_src(pickup.kind, CRATE_COL_INTACT), dest, Vec2::new(0.0, 0.0), 0.0, shadow);
        }
        crate::pyro::draw(c, &drop.dust);
        if drop.visible {
            let dest = Rectangle::new(
                crate::pyro::snap(at.x - drop.w * 0.5) as f32,
                crate::pyro::snap(at.y + CRATE_CELL * 0.5 - drop.h - drop.lift) as f32,
                drop.w,
                drop.h,
            );
            c.blit(Sheet::Crates, crate_src(pickup.kind, CRATE_COL_INTACT), dest, Vec2::new(0.0, 0.0), 0.0, Color::WHITE);
        }
        return;
    }
    draw_crate(c, pickup.kind, at, crate::crate_fx::glint_col(time, at, &t), shadows, Color::WHITE);
}

/// The flames on a burning crate (`Pickup::burn`): tongues off its lid in
/// the effects language (`pyro::tongues`), leaning with the wind, their
/// light among the shapes for the additive pass. Empty for one not burning.
pub fn crate_flames(pickup: &Pickup, time: f32) -> Vec<crate::pyro::Shape> {
    let mut out = Vec::new();
    if pickup.burn.is_none() {
        return out;
    }
    let t = crate::tuning::tuning();
    let at = pickup.position;
    let seed = crate::blast::seed_at(at, 49);
    let lean = crate::pyro::smoke_lean(&t, at, time);
    crate::pyro::tongues(&mut out, Position::new(at.x, at.y + 8.0), 12.0, t.ground_fire_height_px, 3, seed, time, lean, 1.0);
    out
}

/// One crate standing on its cell in sheet column `col`, its shadow first
/// when `shadows`; `tint` multiplies it (a white with alpha fades it).
pub fn draw_crate(c: &mut impl Canvas, kind: PickupKind, at: Position, col: usize, shadows: bool, tint: Color) {
    let origin = Vec2::new(CRATE_CELL * 0.5, CRATE_CELL * 0.5);
    if shadows {
        let t = crate::tuning::tuning();
        let dest = Rectangle::new(at.x + t.shadow_dir_x * t.obstacle_shadow_offset, at.y + t.shadow_dir_y * t.obstacle_shadow_offset, CRATE_CELL, CRATE_CELL);
        let shadow = Color::new(0, 0, 0, (t.obstacle_shadow_opacity * tint.a as f32) as u8);
        c.blit(Sheet::Crates, crate_src(kind, col), dest, origin, 0.0, shadow);
    }
    let dest = Rectangle::new(at.x, at.y, CRATE_CELL, CRATE_CELL);
    c.blit(Sheet::Crates, crate_src(kind, col), dest, origin, 0.0, tint);
}

/// The symbol on its own, `size` px square centred on `at`
/// (`PICKUP_GLYPH_CELL` is its sheet's own scale, 2 px a design pixel).
pub fn draw_glyph(c: &mut impl Canvas, kind: PickupKind, at: Position, size: f32, tint: Color) {
    let dest = Rectangle::new(crate::pyro::snap(at.x - size * 0.5) as f32, crate::pyro::snap(at.y - size * 0.5) as f32, size, size);
    c.blit(Sheet::PickupGlyphs, glyph_src(kind), dest, Vec2::new(0.0, 0.0), 0.0, tint);
}


#[cfg(test)]
mod reach_tests {
    use super::*;

    fn pickup() -> Pickup {
        Pickup::placed(PickupKind::Health, Position::new(100.0, 100.0))
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
