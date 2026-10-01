use crate::tuning::tuning;
use clap::ValueEnum;
use rapier2d::prelude::RigidBodyHandle;
use serde::{Deserialize, Serialize};
use crate::math::{Color, Rectangle, Vec2};

use crate::canvas::{Canvas, Sheet};
use crate::laser::LaserVariant;
use crate::pickup::PickupKind;
use crate::plasma::PlasmaVariant;
use crate::shell::Owner;
use crate::{
    MAX_DAMAGE,
    MISSILE_TUBE_OFFSETS,
    Position,
    TANK_BROKEN_TURRET_COL,
    TANK_DAMAGE_TIERS,
    TANK_FRAME_SIZE,
    TANK_HULL_BBOX_BY_ROW,
    TANK_HULL_FRACTION,
    TANK_MODULE_FLAME_COL,
    TANK_MODULE_LASER_COL,
    TANK_MODULE_MINIGUN_COL,
    TANK_MODULE_MISSILES_COL,
    TANK_MODULE_PLASMA_COL,
    TANK_MOVE_BBOX_FRACTION,
    TANK_PIVOT_REAR_FRACTION,
    TANK_ROWS_PER_TEAM,
    TANK_SPRITE_SIZE,
    TANK_TEAM_BLOCKS,
    TANK_TRACK_FRAMES,
    TANK_TURRET_BBOX_BY_ROW,
    TANK_TURRET_COL,
    TANK_TURRET_POSES,
    TANK_WRECK_COLS,
};

/// The four movement/facing directions. rotation 0 == up, clockwise positive,
/// matching the sprite orientation and shell-spawn math.
#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// All four, in `index` order.
    pub const ALL: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

    /// Position in `ALL`, for per-direction tables.
    pub fn index(self) -> usize {
        match self {
            Dir::Up => 0,
            Dir::Down => 1,
            Dir::Left => 2,
            Dir::Right => 3,
        }
    }

    /// Hull rotation in degrees for this direction.
    pub fn rotation(self) -> f32 {
        match self {
            Dir::Up => 0.0,
            Dir::Right => 90.0,
            Dir::Down => 180.0,
            Dir::Left => 270.0,
        }
    }

    /// Inverse of `rotation`: the direction a hull angle in degrees faces,
    /// any full turns folded away; `None` for an angle that is not one of
    /// the four (within a degree), which a `Tank::rotation` never is.
    pub fn from_rotation(deg: f32) -> Option<Dir> {
        let deg = deg.rem_euclid(360.0);
        Dir::ALL.into_iter().find(|d| {
            let diff = (deg - d.rotation()).abs();
            diff < 1.0 || diff > 359.0
        })
    }

    /// Unit movement vector (screen space: +x right, +y down).
    pub fn vec(self) -> Vec2 {
        match self {
            Dir::Up => Vec2::new(0.0, -1.0),
            Dir::Down => Vec2::new(0.0, 1.0),
            Dir::Left => Vec2::new(-1.0, 0.0),
            Dir::Right => Vec2::new(1.0, 0.0),
        }
    }

    /// Lower-case name, the form `parse` accepts back (tooling/JSON).
    pub fn name(self) -> &'static str {
        match self {
            Dir::Up => "up",
            Dir::Down => "down",
            Dir::Left => "left",
            Dir::Right => "right",
        }
    }

    /// Inverse of `name` (case-insensitive); `None` for anything else.
    pub fn parse(s: &str) -> Option<Dir> {
        match s.trim().to_ascii_lowercase().as_str() {
            "up" => Some(Dir::Up),
            "down" => Some(Dir::Down),
            "left" => Some(Dir::Left),
            "right" => Some(Dir::Right),
            _ => None,
        }
    }

    /// Cardinal direction from `from` toward `to`, choosing the dominant axis.
    pub fn toward(from: Position, to: Position) -> Dir {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        if dx.abs() >= dy.abs() {
            if dx >= 0.0 { Dir::Right } else { Dir::Left }
        } else if dy >= 0.0 {
            Dir::Down
        } else {
            Dir::Up
        }
    }
}

/// The twelve chassis rows of `scifi_tanks_sheet.png`, by name and in row
/// order (see docs/SPRITESHEET_SPEC.md §4) - the same order as
/// `tuning::TANK_NAMES` and every `TANK_*_BY_ROW` table in `lib.rs`, which
/// `chassis_name_order` locks in. Lets a chassis be named rather than
/// addressed by row index: `--tank titan` on either binary, a map's
/// `tank = "titan"` key (`map::MapFile::tank`), and the `player_tank`
/// tuning knob all resolve through here to a `row`.
///
/// `ValueEnum` renders each variant as its lowercase name on the CLI
/// (`Titan` -> `titan`), so the list doubles as `--help`'s own reference,
/// and serde reads/writes that same spelling in a map's TOML.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum TankKind {
    Scout,
    Assault,
    Breaker,
    Longbow,
    Flak,
    Wraith,
    Warden,
    Ravager,
    Glacier,
    Obelisk,
    Titan,
    Leviathan,
}

impl TankKind {
    /// Every chassis, in row order - `ALL[i].row() == i as i32`.
    pub const ALL: [TankKind; 12] = [
        TankKind::Scout,
        TankKind::Assault,
        TankKind::Breaker,
        TankKind::Longbow,
        TankKind::Flak,
        TankKind::Wraith,
        TankKind::Warden,
        TankKind::Ravager,
        TankKind::Glacier,
        TankKind::Obelisk,
        TankKind::Titan,
        TankKind::Leviathan,
    ];

    /// This chassis's row index into `scifi_tanks_sheet.png` - the variant's
    /// own declaration position, which is the sheet's row order.
    pub fn row(self) -> i32 {
        self as i32
    }

    /// The chassis drawn from `row`, or `None` for a row outside the sheet.
    pub fn from_row(row: i32) -> Option<TankKind> {
        usize::try_from(row).ok().and_then(|i| Self::ALL.get(i).copied())
    }

    /// This chassis's lowercase name - the spelling `--tank`, a map's
    /// `tank` key and the dev panel's tank tables all use.
    pub fn name(self) -> &'static str {
        crate::tuning::TANK_NAMES[self.row() as usize]
    }
}

/// A twin-barrel chassis's second shell, waiting to fire a beat after the
/// first - see `Tank::pending_shot`.
#[derive(Clone, Copy)]
pub struct PendingShot {
    /// Seconds remaining until this shell fires.
    pub timer: f32,
    /// Same off-aim deflection as the first shell (see `Shell::spawn`'s
    /// `aim_offset` param) - a misfire skews both rounds identically.
    pub aim_offset: f32,
    /// This barrel's lateral offset (see `Shell::spawn`'s `lateral_offset`
    /// param) - the opposite side from the first shell's barrel.
    pub lateral_offset: f32,
}

/// A twin-barrel chassis's second plasma bolt, waiting to fire a beat after
/// the first - see `Tank::pending_plasma_shot`. Identical shape to
/// `PendingShot`, just for `plasma::Plasma` instead of `Shell` - kept as its
/// own type (not a shared generic) since the two weapons' fire-dispatch
/// sites in `simulation::Game::update` are already separate match arms with
/// nothing else in common to factor through.
#[derive(Clone, Copy)]
pub struct PendingPlasmaShot {
    /// Seconds remaining until this bolt fires.
    pub timer: f32,
    /// Same off-aim deflection as the first bolt (see `PendingShot::aim_offset`).
    pub aim_offset: f32,
    /// This barrel's lateral offset (see `PendingShot::lateral_offset`).
    pub lateral_offset: f32,
}

/// A minigun burst in progress: `bullets_remaining` more bullets queued to
/// fire at MINIGUN_BULLET_DELAY_SECONDS spacing after the one that just
/// fired - see `Tank::minigun_burst`. Generalizes `PendingShot`'s "one
/// queued extra shot" to "N queued burst bullets" using the identical
/// tick-once-per-frame shape.
#[derive(Clone, Copy)]
pub struct MinigunBurst {
    pub bullets_remaining: u32,
    /// Seconds remaining until the next queued bullet fires.
    pub timer: f32,
    /// Same point-blank misfire skew as `PendingShot::aim_offset` - rolled
    /// once when the burst starts and shared by every bullet in it; each
    /// bullet additionally gets its own fresh MINIGUN_BULLET_SPREAD_DEG
    /// jitter on top of this at the moment it actually fires (see
    /// `simulation::fire_bullet`).
    pub aim_offset: f32,
}

/// A seeker-missile volley in progress: `missiles_remaining` more to leave
/// the launcher, `timer` until the next one, `next_tube` the tube it leaves
/// from - see `Tank::missile_volley`. The `MinigunBurst` shape, one missile per
/// tube; the tubes are fired in order and wrap back to the first for the
/// next salvo (`missile_salvos`).
#[derive(Clone, Copy)]
pub struct MissileVolley {
    pub missiles_remaining: u32,
    pub timer: f32,
    pub next_tube: u8,
}

/// Which weapon a tank's next trigger-pull fires - see `Tank::active_weapon`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveWeapon {
    Laser,
    Plasma,
    Minigun,
    Missiles,
    Flamethrower,
    Shell,
}

impl ActiveWeapon {
    /// Lower-case name for tooling/JSON (`Event::Fired`, the dev server).
    pub fn name(self) -> &'static str {
        match self {
            ActiveWeapon::Laser => "laser",
            ActiveWeapon::Plasma => "plasma",
            ActiveWeapon::Minigun => "minigun",
            ActiveWeapon::Missiles => "missiles",
            ActiveWeapon::Flamethrower => "flamethrower",
            ActiveWeapon::Shell => "shell",
        }
    }
}

pub struct Tank {
    /// Which of the 12 tank archetypes in scifi_tanks_sheet.png this tank
    /// draws (see TANK_VARIANTS/TANK_SPRITE_ORDER in simulation.rs). The hull
    /// and turret layers' columns within that row depend on this tank's
    /// current animation/damage state - see `hull_col`/`turret_col`.
    pub row: i32,
    /// Row in shells.png this tank's shells are drawn from (0..SHELL_VARIANTS)
    /// - matched to this tank's chassis (size class, accent colour) via
    /// TANK_SHELL_VARIANT_BY_ROW, set once at spawn (see Game::init) and
    /// fixed for this tank's whole life - every shell it ever fires,
    /// including both independent shots from a twin-barrel chassis (see
    /// `pending_shot`), uses this same single-barrel row.
    pub shell_variant: i32,
    /// A twin-barrel chassis's second shell, queued to fire
    /// TANK_TWIN_SHOT_DELAY_SECONDS after the first so the two rounds read
    /// as one barrel, then the other, rather than appearing in the same
    /// instant. `None` the rest of the time; a single-barrel chassis never
    /// sets this (see `TANK_BARREL_LATERAL_OFFSET_BY_ROW`). Ticked down and
    /// resolved once per frame in `Game::update`, at the same site this
    /// tank's own fire input is handled.
    pub pending_shot: Option<PendingShot>,
    /// A twin-barrel chassis's second plasma bolt, queued the same way
    /// `pending_shot` queues a shell's second barrel - see
    /// `PendingPlasmaShot`. `None` the rest of the time; a single-barrel
    /// chassis never sets this.
    pub pending_plasma_shot: Option<PendingPlasmaShot>,
    /// A minigun burst in progress, queued the same way `pending_shot`
    /// above queues a twin-barrel chassis's second shell - generalized from
    /// "one queued extra shot" to "N queued burst bullets". `None` the rest
    /// of the time. Ticked/resolved once per frame in `Game::update`, at
    /// the same call sites `pending_shot` is already ticked for player and
    /// enemy.
    pub minigun_burst: Option<MinigunBurst>,
    /// A seeker-missile volley in progress (`MissileVolley`), ticked with
    /// the other queued shots so a volley always completes unless the tank
    /// is wrecked. `None` the rest of the time.
    pub missile_volley: Option<MissileVolley>,
    /// How many of the launcher's tubes read empty right now (0..=4, the
    /// missile module's cell): counts up as a volley leaves, back down as
    /// the reload runs (`tick_missile_pod`). Presentation only.
    pub missile_tubes_empty: u8,
    /// Where on its engine deck this hull smokes and burns
    /// (0..DAMAGE_VARIANTS, `damage_stage::engine_deck`). Rolled once at
    /// spawn (see Game::init) and fixed for the tank's whole life, so it
    /// burns in the same place however the fight goes.
    pub damage_variant: i32,
    /// Center position on screen (pixels). A read-back mirror of `body`'s
    /// physics transform, synced once per frame after the physics world
    /// steps (see `Game::update`) - nothing else should write this by hand.
    pub position: Position,
    /// Facing angle in degrees. Snaps instantly on a direction change -
    /// physics (`Game::drive_tank`), aiming (`Shell::spawn`) and track
    /// heading all key off this and none of it should lag input. For the
    /// on-screen hull animation, see `visual_rotation` instead.
    pub rotation: f32,
    /// Sprite-only facing angle in degrees, eased toward `rotation` each
    /// frame (see `ease_visual_rotation`/`TANK_VISUAL_TURN_SPEED_DEG`)
    /// instead of snapping with it, so a turn visibly swings the hull over a
    /// few frames. Read only by `draw_tank`/`draw_tank_shadow` - nothing
    /// gameplay-relevant should ever key off this.
    pub visual_rotation: f32,
    /// Turret-only sprite facing angle in degrees, eased toward `rotation`
    /// independently of `visual_rotation` (see
    /// `ease_turret_visual_rotation`/`TANK_TURRET_VISUAL_TURN_SPEED_DEG`) -
    /// faster than the hull's own ease, so the turret visibly leads a turn
    /// while the hull swings around to catch up. Read only by `draw_tank`/
    /// `draw_tank_shadow`.
    pub turret_visual_rotation: f32,
    /// Where this tank's ground ring (the shield ring, the player's white
    /// marker - see `draw_ground_ring`) is drawn. A sleepy follower of
    /// `position`: it has its own `ring_velocity` and chases the hull as a
    /// spring-damper (`ease_ring_position`/`tank_ring_spring_hz`/
    /// `tank_ring_damping`), so it hangs back when the tank sets off, trails
    /// further the faster the hull moves, then swings in and settles once
    /// the tank stops. Snaps whenever the hull is teleported or spawned far
    /// from it. Presentation-only, like `visual_rotation`.
    pub ring_position: Position,
    /// The ground ring's own velocity (px/s) - the inertia that makes it
    /// lag and catch up rather than track the hull instantly.
    pub ring_velocity: Vec2,
    /// Seconds accumulated toward the minigun module's next "hot barrel"
    /// cell (see `module_cols`), advanced while `minigun_burst` is active
    /// (see `tick_minigun_spin`) and held in place - not reset to 0 - the
    /// rest of the time, so the module doesn't visually snap back to frame 0
    /// between bursts. Wrapped to `minigun_cycle_seconds * 3.0` (one full
    /// lap of the 3 frames) rather than growing unbounded.
    ///
    /// This deliberately drives a discrete cell swap, not a continuous
    /// rotation: the barrels point along the ground plane toward the
    /// target, so their real rotation axis is edge-on to this game's
    /// top-down camera - spinning the sprite in the screen plane would read
    /// as a helicopter rotor seen from above. Cycling which barrel reads as
    /// freshly fired fakes the rounds cycling through, correctly for this
    /// camera angle.
    pub minigun_cycle_timer: f32,
    /// The hull's track frame (0..TANK_TRACK_FRAMES), advanced by distance
    /// driven (`simulation::lay_tracks`) at every live damage tier - see
    /// `hull_col`.
    pub hull_frame: i32,
    /// The turret's recoil cell (0..TANK_TURRET_POSES): 0 at rest, 1 the
    /// first (or only) barrel kicked back, 2 the second barrel back or a
    /// single one returning. Set by `kick` where a shell or plasma bolt
    /// leaves the main gun, stepped by `tick_recoil`. Presentation only.
    pub recoil_pose: u8,
    /// Seconds the current recoil cell has left (`tank_recoil_seconds`).
    pub recoil_timer: f32,
    /// A twin gun's second barrel kicks this many seconds after the first -
    /// when its second shell leaves (`tank_twin_shot_delay_seconds`).
    pub recoil_second_in: Option<f32>,
    /// The last kick was a plasma bolt: the plasma module shows its coils
    /// firing through the recoil.
    pub recoil_plasma: bool,
    /// Seconds the laser module shows its lens firing (`kick_laser`).
    pub laser_flash_timer: f32,
    /// World px of travel accumulated toward the next `hull_frame` advance -
    /// see `simulation::lay_tracks`. Deliberately separate from
    /// `track_accum` below: that one paces the ground-decal tread marks in
    /// track.rs, an unrelated system with its own spacing: reusing it here
    /// would tie two independently-tuned animations together.
    pub hull_anim_accum: f32,
    /// Which of TANK_WRECK_COLS this tank uses once it becomes a wreck -
    /// `None` until then. Rolled once, the frame `is_wreck()` first becomes
    /// true (see `simulation::Game::update`, right where `tick_wreck` is
    /// called), and kept for the rest of the tank's lifetime rather than
    /// re-rolled each frame, so a field of wrecks shows genuine variety
    /// instead of flickering between variants.
    pub wreck_col: Option<i32>,
    /// World pixels per design pixel: the 32 px frame (`size`) and the
    /// 40 px sprite cell (`sprite_size`) are both drawn this many times up.
    pub scale: f32,
    /// Per-tank multiplier on the live base speed (`tuning().tank_speed`
    /// for the player, `tuning().enemy_speed` for an enemy - see
    /// `base_speed`): 1.0 for the player, an enemy's spawn-rolled
    /// `enemy_speed_variance` factor. Stored as a factor rather than an
    /// absolute px/s so dragging the speed knob mid-round moves every tank
    /// on the field, not just the next one to spawn.
    pub speed_scale: f32,
    /// Accumulated damage, 0 (pristine) .. MAX_DAMAGE (destroyed wreck).
    pub damage: f32,
    /// Remaining shells this tank can fire before it must recharge.
    pub shells_ammo: i32,
    /// Remaining laser charges (see `pickup::PickupKind::Laser`,
    /// `laser.rs`). While this is the live weapon (front of the stocked
    /// `weapon_queue` - see `active_weapon`) and positive, firing consumes
    /// one charge and resolves an instant beam hit instead of a normal
    /// shell (see `simulation.rs`'s fire dispatch); a nonzero balance sits
    /// waiting while an earlier-queued weapon holds the trigger.
    pub laser_charges: i32,
    /// Which `laser::LaserVariant` the current charge batch fires as -
    /// rolled fresh on each `PickupKind::Laser` pickup (see
    /// `LASER_BLUE_PICKUP_CHANCE`), meaningless while `laser_charges == 0`.
    pub laser_variant: LaserVariant,
    /// Remaining minigun ammo (see `pickup::PickupKind::Minigun`,
    /// `bullet.rs`). Pickup-only, no passive regen - mirrors `laser_charges`
    /// exactly. While this is the live weapon and positive, firing starts a
    /// burst of MINIGUN_BURST_SIZE individually-simulated `bullet::Bullet`s
    /// instead of a normal shell - see `Tank::active_weapon`.
    pub minigun_ammo: i32,
    /// Remaining plasma ammo (see `pickup::PickupKind::Plasma`, `plasma.rs`).
    /// Pickup-only, no passive regen - mirrors `minigun_ammo`/`laser_charges`
    /// exactly. While this is the live weapon and positive, firing shoots a
    /// `plasma::Plasma` bolt from the barrel instead of a normal shell -
    /// see `Tank::active_weapon`.
    pub plasma_ammo: i32,
    /// Which `plasma::PlasmaVariant` the current charge batch fires as -
    /// rolled fresh on each `PickupKind::Plasma` pickup (see
    /// `PLASMA_PURPLE_PICKUP_CHANCE`), meaningless while `plasma_ammo == 0`.
    /// Same mechanism as `laser_variant`.
    pub plasma_variant: PlasmaVariant,
    /// Remaining seeker missiles (`pickup::PickupKind::Missiles`,
    /// `missile.rs`). Pickup-only, one per missile - a full volley costs
    /// `missile_volley_size`. While this is the live weapon and positive,
    /// a trigger pull fires a volley instead of a shell.
    pub missile_ammo: i32,
    /// Flamethrower fuel in seconds of burn (`pickup::PickupKind::
    /// Flamethrower`, docs/flamethrower-prd.md). Pickup-only; drained by
    /// `dt` every frame the trigger is held with the flamethrower live.
    pub flame_fuel: f32,
    /// True while the trigger has been held on the flamethrower since the
    /// last frame it was not: `Event::Fired` is recorded once per hold.
    pub flame_held: bool,
    /// Seconds this tank keeps burning after a flame stream touched it
    /// (`flame_afterburn_seconds`); `flame_afterburn_dps` a second while
    /// positive. Set, never added to, so re-contact resets it.
    pub burn_timer: f32,
    /// The seat whose shot, beam, flame, missile or ram last damaged this
    /// tank: who its wreck is credited to on the end screen
    /// (`Game::round_stats`). `None` until a seat touches it, and always on
    /// a seat's own tank.
    pub last_hit_by: Option<u8>,
    /// Seconds of wet tread marks left after wading (docs/water.md):
    /// refreshed every frame the hull is in water, counted down by
    /// `tick_timers`, read by `lay_tracks`.
    pub wet_timer: f32,
    /// Seconds left of a coat of ooze from a bio slush tower
    /// (docs/defence-towers-prd.md section 6): while positive the tank
    /// corrodes at `bio_slime_dps` and drives at `bio_slime_speed_factor`
    /// of its pace. Set, never added to; water washes it off.
    pub slime_timer: f32,
    /// FIFO queue of this tank's collected special weapons. The inventory
    /// rule: the weapon at the front keeps firing until its own ammo runs
    /// dry - a fresh pickup never interrupts it, it lines up *behind* (see
    /// `enqueue_weapon`, called from `Game::update`'s pickup-collection
    /// block and the armed-spawn roll in `Game::init`) - then the next
    /// queued weapon takes over, and only once every queued special is
    /// spent does the trigger fall back to the default, always-recharging
    /// shell cannon. `active_weapon` derives all of that by scanning this
    /// queue; spent entries are skipped there and pruned lazily on the next
    /// pickup (`enqueue_weapon`) rather than eagerly popped at the many
    /// places ammo can hit zero. Ammo/Health/SpeedUp pickups never touch
    /// this: an ammo crate is a resupply for the shell cannon, not a queue
    /// entry. At most one entry per weapon kind ever sits here (a repeat
    /// pickup of a still-stocked kind just tops up its ammo counter and
    /// keeps its place in line), so the queue never exceeds the three
    /// special kinds.
    pub weapon_queue: Vec<ActiveWeapon>,
    /// Seconds remaining on a `pickup::PickupKind::SpeedUp` boost - while
    /// positive, `effective_speed` scales top speed by
    /// SPEED_BOOST_MULTIPLIER. Set (not added to) on pickup, so a fresh
    /// pickup refreshes the duration instead of stacking with an
    /// already-active boost - see `PickupKind::SpeedUp`'s doc comment.
    pub speed_boost_timer: f32,
    /// Absorption left in a `pickup::PickupKind::Shield` - a pool of damage
    /// points, not a clock. While positive `take_damage` spends this instead
    /// of health and `draw_tank_shield` draws the rainbow ring; at zero the
    /// shield shatters and does not come back until another pickup. Set (not
    /// added to) on pickup, same refill-not-stack rule as
    /// `speed_boost_timer`.
    ///
    /// Projectiles never reach `take_damage` - they bounce off in
    /// `Game::resolve_projectiles` and are charged `shield_deflect_cost_factor`
    /// times their own damage there, so both seams spend this field.
    /// The fraction of its commanded speed this tank was last actually
    /// driven at - `Intent::speed_scale()`, recorded by `Game::drive_tank`.
    ///
    /// Exists so `TankSnapshot::commanded_velocity` can report what was
    /// really asked of the tank. `Tank::velocity` stays the *unthrottled*
    /// cardinal on purpose, because `motion_snapshot` feeds it to the AI's
    /// own predictive avoidance and the throttle is not the AI's business;
    /// but the probe windows commanded-vs-achieved for `low-progress` and
    /// `wall-grind`, and a full-magnitude command against a deliberately
    /// throttled body is a false positive on a zero-ceiling kind.
    pub throttle: f32,
    pub shield_hp: f32,
    /// Seconds left before a live shield starts refilling. Reset to
    /// `shield_recharge_delay_seconds` every time the shield absorbs
    /// anything, so only a tank that breaks contact recovers - see
    /// `Game::tick_timers`.
    pub shield_recharge_delay: f32,
    /// Set by `spend_shield` on the frame the shield shatters, drained and
    /// cleared by `Game::drain_shield_breaks` at the end of the frame's
    /// damage phases.
    ///
    /// A flag rather than a return value the callers act on, because the
    /// shield is spent at eight different seams - ram (both tanks), blasts,
    /// the flame cone, its afterburn, oil fire, the frog and the projectile
    /// deflect - and `take_damage` has no `Frame` to push an event onto.
    /// Latching here means every one of them announces the break, instead of
    /// the two that happened to be written to.
    pub shield_broke: bool,
    /// Seconds accumulated toward recharging the next shell.
    pub recharge_timer: f32,
    /// Seconds remaining before this tank may fire again (player only - see
    /// PLAYER_FIRE_INTERVAL; enemies are gated separately by Ai's own
    /// fire_timer). Ticked down alongside ram_cooldown every frame regardless
    /// of owner, since it's harmless/unused idle state for enemies.
    pub fire_cooldown: f32,
    /// Seconds remaining before this tank can take ramming damage again.
    pub ram_cooldown: f32,
    /// Seconds before this tank may enter a portal again
    /// (`portal_cooldown_seconds`, set the frame it arrives through one;
    /// `Game::portal_phase`). Ticked in `Game::tick_timers`, but only
    /// while the tank is outside every portal's trigger radius.
    pub portal_cooldown: f32,
    /// Seconds left in this tank's hit window: reset to
    /// `health_ring_hit_seconds` by `mark_hit` whenever it takes damage
    /// (shell, ram, explosion splash, frog bite), ticked down every frame
    /// alongside `fire_cooldown`/`ram_cooldown` (`Game::update`). An enemy's
    /// health ring shows while it runs, fading over the last
    /// `health_ring_hit_fade_seconds` (`enemy_health_ring_visibility`); the
    /// player's ring is always on, so for the player this is bookkeeping.
    pub hit_flash_timer: f32,
    /// Seconds spent as a wreck. Once it exceeds WRECK_BURN_SECONDS the fire
    /// dies out and the tank becomes a static charred "dead" hulk.
    pub wreck_timer: f32,
    /// Wave rounds only: seconds until this wreck is removed from the field
    /// (`wave_wreck_despawn_seconds`, armed by `Game::despawn_wrecks` the
    /// frame the tank becomes a wreck). `None` for a live tank and in band
    /// rounds, where wrecks stay for the whole round. The last second is
    /// the fade `alpha` reports.
    pub despawn_timer: Option<f32>,
    /// Distance travelled (pixels) since the last track mark was dropped.
    pub track_accum: f32,
    /// Where this hull stood at the last `Game::tick_presentation`, the
    /// `before` its tread marks are laid from on a client replica
    /// (docs/online-coop-prd.md section 4.5). `None` in a local round,
    /// whose marks come off the physics step instead
    /// (`Game::sync_tanks_and_ram`).
    pub track_from: Option<Position>,
    /// Number of track marks this tank has laid this round - the phase input
    /// to its track wobble (see `track_wobble_phase`); incremented once per
    /// mark in `Game::lay_tracks`.
    pub track_mark_count: u32,
    /// This tank's track-wobble amplitude in degrees, rolled once at spawn
    /// (see TRACK_WOBBLE_AMP_MIN_DEG/MAX_DEG) and fixed for the round -
    /// how far each mark's rotation swings side to side from the tank's
    /// actual heading.
    pub track_wobble_amp: f32,
    /// This tank's track-wobble angular frequency, in radians per mark
    /// (derived from a randomized wavelength at spawn - see
    /// TRACK_WOBBLE_WAVELENGTH_MIN/MAX) - how tight the wobble's cycles are.
    pub track_wobble_freq: f32,
    /// This tank's track-wobble phase offset in radians, rolled once at
    /// spawn, so tanks that happen to share a similar amplitude/frequency
    /// don't wobble in lockstep.
    pub track_wobble_phase: f32,
    /// Fixed per-tank multiplier on track mark scale (see
    /// TRACK_SCALE_JITTER), rolled once at spawn.
    pub track_scale_jitter: f32,
    /// Commanded velocity this frame (pixels per second): the movement
    /// direction times speed, or zero when not moving. Set by `control` and
    /// read by the AI's predictive collision avoidance, and by
    /// `Game::drive_tank` to derive how much of the physics body's actual
    /// velocity is "ours" versus residual momentum from a ram/explosion
    /// impulse (see that function).
    pub velocity: Vec2,
    /// This tank's rapier rigid body, once spawned into the physics world
    /// (see `Game::init`/`physics::Physics::spawn_tank`).
    pub body: Option<RigidBodyHandle>,
    /// Who this tank is, set once at spawn (`Game::init`): `Owner::Player(i)`
    /// for human player `i`, `Owner::Enemy(slot)` for an enemy. Identifies
    /// the tank as a projectile owner so a shot never hits its own shooter,
    /// and decides the side it fights on. See `owner_slot` for the
    /// numbering the tools and events use.
    pub owner: Owner,
}

impl Default for Tank {
    fn default() -> Self {
        Self {
            row: 0,
            shell_variant: 0,
            pending_shot: None,
            pending_plasma_shot: None,
            minigun_burst: None,
            missile_volley: None,
            missile_tubes_empty: 0,
            damage_variant: 0,
            position: Position::default(),
            rotation: 0.0,
            visual_rotation: 0.0,
            turret_visual_rotation: 0.0,
            ring_position: Position::default(),
            ring_velocity: Vec2::new(0.0, 0.0),
            minigun_cycle_timer: 0.0,
            hull_frame: 0,
            recoil_pose: 0,
            recoil_timer: 0.0,
            recoil_second_in: None,
            recoil_plasma: false,
            laser_flash_timer: 0.0,
            hull_anim_accum: 0.0,
            wreck_col: None,
            scale: 2.0, // 3.0,
            speed_scale: 1.0,
            damage: 0.0,
            shells_ammo: tuning().max_shells,
            laser_charges: 0,
            flame_fuel: 0.0,
            flame_held: false,
            burn_timer: 0.0,
            last_hit_by: None,
            slime_timer: 0.0,
            wet_timer: 0.0,
            laser_variant: LaserVariant::Red,
            minigun_ammo: 0,
            plasma_ammo: 0,
            plasma_variant: PlasmaVariant::Teal,
            missile_ammo: 0,
            weapon_queue: Vec::new(),
            speed_boost_timer: 0.0,
            throttle: 1.0,
            shield_hp: 0.0,
            shield_recharge_delay: 0.0,
            shield_broke: false,
            recharge_timer: 0.0,
            fire_cooldown: 0.0,
            ram_cooldown: 0.0,
            portal_cooldown: 0.0,
            hit_flash_timer: 0.0,
            wreck_timer: 0.0,
            despawn_timer: None,
            track_accum: 0.0,
            track_from: None,
            track_mark_count: 0,
            track_wobble_amp: 0.0,
            track_wobble_freq: 0.0,
            track_wobble_phase: 0.0,
            track_scale_jitter: 1.0,
            velocity: Vec2::new(0.0, 0.0),
            body: None,

            owner: Owner::Player(0),
        }
    }
}

impl Tank {
    /// This tank's gameplay size: the 32 px frame its boxes are measured
    /// in, at `scale` - what avoidance, spawn clearance and the ground
    /// rings are sized from. The sprite is bigger (`sprite_size`).
    pub fn size(&self) -> f32 {
        TANK_FRAME_SIZE * self.scale
    }

    /// The side of the sprite's square cell on screen: the 40 px cell at
    /// `scale`, room for barrels, antennas and modules round the frame.
    pub fn sprite_size(&self) -> f32 {
        TANK_SPRITE_SIZE * self.scale
    }

    /// True once the tank has taken maximum damage (a burning wreck).
    pub fn is_wreck(&self) -> bool {
        self.damage >= MAX_DAMAGE
    }

    /// Remaining health as a fraction of `MAX_DAMAGE`, 1 pristine to 0
    /// wrecked - the one fraction the health ring and the debug snapshot
    /// read.
    pub fn health_fraction(&self) -> f32 {
        (1.0 - self.damage / MAX_DAMAGE).clamp(0.0, 1.0)
    }

    /// Remaining health in whole points, rounded to the nearest: 0 only
    /// for a wreck, at least 1 while the tank lives (so a hull an inch from
    /// death never reads as dead), at most `MAX_DAMAGE`. What travels on
    /// the wire (`net::encode`) and what the picture compares.
    pub fn hull_points(&self) -> u8 {
        if self.is_wreck() {
            return 0;
        }
        let points = (MAX_DAMAGE - self.damage).round().clamp(1.0, MAX_DAMAGE.min(255.0));
        points as u8
    }

    /// True while a rainbow shield is active (see `shield_hp`).
    /// Whether collecting `kind` would actually do this tank any good.
    ///
    /// **Enemies only take what they need.** A pack of enemies that hoovers
    /// up every crate it drives past - at full health, with a full magazine,
    /// already stocked with the weapon - strips the field of everything the
    /// player was going to use, for no benefit to itself. It reads as spite
    /// rather than as intelligence, and it is not a difficulty knob: it takes
    /// resources away from the player without giving the enemy anything, so
    /// the only thing it tunes is how annoying the round is.
    ///
    /// A **player** always collects. Choosing to take something you do not
    /// strictly need - denying it to the other side, topping up before a
    /// push, grabbing a shield you will want in ten seconds - is a decision
    /// the person at the controls is entitled to make. The asymmetry is the
    /// point.
    ///
    /// This is deliberately the *same* predicate the behaviour tree's
    /// `seek_*` tiers gate on (`ai::build`, tiers 5.5-5.9), and they call it
    /// rather than repeating the conditions. If collection were ever stricter
    /// than seeking, a tank would drive to a pickup it then refused to take
    /// and sit on top of it forever - so the two agreeing is a correctness
    /// requirement, not tidiness.
    pub fn wants_pickup(&self, kind: PickupKind) -> bool {
        if self.is_player() {
            return true;
        }
        match kind {
            PickupKind::Health => self.damage > 0.0,
            PickupKind::Ammo => self.shells_ammo < tuning().max_shells,
            PickupKind::Laser => self.laser_charges <= 0,
            PickupKind::Plasma => self.plasma_ammo <= 0,
            PickupKind::Minigun => self.minigun_ammo <= 0,
            PickupKind::Missiles => self.missile_ammo <= 0,
            PickupKind::SpeedUp => self.speed_boost_timer <= 0.0,
            PickupKind::Shield => self.shield_hp <= 0.0,
            // Player-only: the fuel tank does nothing for an enemy at all.
            PickupKind::Flamethrower => false,
            // Decided by the frog's own state, not the tank's - see
            // `Game::pickup_phase` and docs/frog-health-pack-prd.md.
            PickupKind::FrogHealth => true,
            // Decided by the towers' state (`Game::tower_pack_wanted`).
            PickupKind::TowerPack => true,
        }
    }

    pub fn is_shielded(&self) -> bool {
        self.shield_hp > 0.0
    }

    /// Coated in ooze.
    pub fn is_slimed(&self) -> bool {
        self.slime_timer > 0.0
    }

    /// The fraction of its pace a coat of ooze leaves this tank: 1 when
    /// clean, so a round without towers multiplies by exactly one.
    pub fn slime_pace(&self) -> f32 {
        if self.is_slimed() { tuning().bio_slime_speed_factor } else { 1.0 }
    }

    /// How much of a shield is left, 0..=1: `shield_hp` over
    /// `shield_capacity` (what a pickup or a spawn roll sets it to), clamped
    /// so a pool pushed past the knob still reads as full. The HUD gauge
    /// (`hud::PlayerHud`) and the rainbow ring sweep (`draw_tank_shield`)
    /// both read this and neither cares that it now measures absorption
    /// rather than seconds.
    pub fn shield_charge(&self) -> f32 {
        let capacity = tuning().shield_capacity;
        if capacity > 0.0 { (self.shield_hp / capacity).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Spend `amount` of shield on a hit the shield is taking instead of the
    /// hull, and report whether that shattered it (true on the one frame the
    /// pool crosses zero, never again - the edge-trigger shape
    /// `Obstacle::damage` uses). A hit bigger than what is left is absorbed
    /// in full rather than bleeding through, so the last point of shield is
    /// worth having. Resets the recharge delay either way.
    pub fn spend_shield(&mut self, amount: f32) -> bool {
        if !self.is_shielded() {
            return false;
        }
        self.shield_recharge_delay = tuning().shield_recharge_delay_seconds;
        self.shield_hp -= amount.max(0.0);
        if self.shield_hp > 0.0 {
            return false;
        }
        self.shield_hp = 0.0;
        self.shield_broke = true;
        true
    }

    /// The one way damage lands on a tank's *hull*: adds `amount`, capped at
    /// `cap` (MAX_DAMAGE, or one below it for the player's frog bites), and
    /// returns how much actually landed. Callers keep their hit
    /// flash/knockback/alert side effects either way, so an absorbed hit
    /// still visibly lands.
    ///
    /// A live shield takes the hit instead and returns 0: this is the absorb
    /// seam for ram, explosions, flame and its afterburn, oil fire, the frog
    /// and the laser. It is **not** the only seam - shells, bullets and
    /// plasma bounce off in `Game::resolve_projectiles` and never arrive
    /// here, so they are charged to `shield_hp` there instead. Anything that
    /// needs "did the shield just shatter" should call `spend_shield`
    /// directly; this reports only what the hull took.
    pub fn take_damage(&mut self, amount: f32, cap: f32) -> f32 {
        if self.is_shielded() {
            self.spend_shield(amount);
            return 0.0;
        }
        let before = self.damage;
        self.damage = (self.damage + amount).min(cap);
        self.damage - before
    }

    /// Who this tank is as a projectile owner.
    pub fn owner(&self) -> Owner {
        self.owner
    }

    /// This tank's owner slot, the number events, snapshots and the dev
    /// tools address it by: the seats first (`0` for player 1, then one
    /// per further seat), then the enemies, counting up from the first
    /// free number (`Game::first_enemy_slot`). Unique for the round.
    pub fn owner_slot(&self) -> usize {
        self.owner.slot()
    }

    /// A human player, whichever one.
    pub fn is_player(&self) -> bool {
        self.owner.is_player()
    }

    /// Which seat this is, if a player at all.
    pub fn player_index(&self) -> Option<u8> {
        match self.owner {
            Owner::Player(i) => Some(i),
            Owner::Enemy(_) | Owner::Tower { .. } => None,
        }
    }

    /// The row of the sprite sheet this tank draws from: its chassis row
    /// inside the block for its team - the enemy block first, then the four
    /// players' recolours (`TANK_ROWS_PER_TEAM`, `sheet_block`).
    pub fn sheet_row(&self) -> i32 {
        self.row + sheet_block(self.player_index()) * TANK_ROWS_PER_TEAM
    }

    /// How much damage has hurt this tank's mobility, from 1.0 (pristine) down
    /// to DAMAGE_SPEED_FLOOR (about to wreck). Holds close to 1.0 through
    /// light and moderate damage, then falls off harder as damage nears the
    /// max - a limp rather than a linear taper. Scales both top speed
    /// (`effective_speed`) and how fast the tank can reach it
    /// (`TANK_ACCEL_FORCE`/`TANK_DECEL_CURVE_RATE` in `Game::drive_tank`), so a
    /// damaged tank is sluggish to speed up too, not just capped lower.
    pub fn speed_factor(&self) -> f32 {
        let hurt = (self.damage / MAX_DAMAGE).clamp(0.0, 1.0);
        tuning().damage_speed_floor + (1.0 - tuning().damage_speed_floor) * (1.0 - hurt.powf(tuning().damage_speed_curve))
    }

    /// This tank's current top speed, reduced as it takes damage (see
    /// `speed_factor`) and boosted by SPEED_BOOST_MULTIPLIER while
    /// `speed_boost_timer` is positive (see `pickup::PickupKind::SpeedUp`).
    pub fn effective_speed(&self) -> f32 {
        let boost = if self.speed_boost_timer > 0.0 { tuning().speed_boost_multiplier } else { 1.0 };
        self.base_speed() * self.speed_factor() * boost
    }

    /// This tank's undamaged, unboosted top speed (px/s): the live
    /// player/enemy base knob times `speed_scale`.
    pub fn base_speed(&self) -> f32 {
        let t = tuning();
        let base = if self.owner.is_player() { t.tank_speed } else { t.enemy_speed };
        base * self.speed_scale
    }

    /// True once a wreck has finished burning and settled into a dead hulk.
    pub fn is_dead(&self) -> bool {
        self.is_wreck() && self.wreck_timer >= tuning().wreck_burn_seconds
    }

    /// Draw opacity, 0..=1: fully opaque except over the last second of a
    /// wreck's `despawn_timer`, which fades it out before it is removed.
    pub fn alpha(&self) -> f32 {
        self.despawn_timer.map_or(1.0, |t| t.clamp(0.0, 1.0))
    }

    /// White scaled by `alpha` - the tint every layer of the tank's sprite
    /// draws with, so a fading wreck fades as one.
    pub fn tint(&self) -> Color {
        Color::new(255, 255, 255, (255.0 * self.alpha()).round() as u8)
    }

    /// The damage tier the art shows: 0 pristine, one step at each of
    /// `TANK_DAMAGE_TIERS` - scuffed, damaged, critical. A wreck has its
    /// own cells (`hull_col`).
    pub fn damage_tier(&self) -> i32 {
        TANK_DAMAGE_TIERS.iter().filter(|&&at| self.damage >= at).count() as i32
    }

    /// Which sheet column the hull draws from: the rolled wreck once it is
    /// one (`wreck_col`, the first variant on the off chance it is read
    /// before the roll), otherwise its damage tier's run of track frames,
    /// the frame `hull_frame` points at - the tracks roll at every tier.
    pub fn hull_col(&self) -> i32 {
        if self.is_wreck() {
            self.wreck_col.unwrap_or(TANK_WRECK_COLS[0])
        } else {
            self.damage_tier() * TANK_TRACK_FRAMES + self.hull_frame.clamp(0, TANK_TRACK_FRAMES - 1)
        }
    }

    /// Which sheet column the turret draws from: the broken turret on a
    /// wreck, otherwise its damage tier's run of recoil poses, the one
    /// `recoil_pose` holds.
    pub fn turret_col(&self) -> i32 {
        if self.is_wreck() {
            TANK_BROKEN_TURRET_COL
        } else {
            TANK_TURRET_COL + self.damage_tier() * TANK_TURRET_POSES + (self.recoil_pose as i32).min(TANK_TURRET_POSES - 1)
        }
    }

    /// A blown wreck (the first variant): its turret ring is a crater and
    /// the turret lies beside the hull, thrown clear.
    pub fn turret_thrown(&self) -> bool {
        self.is_wreck() && self.wreck_col == Some(TANK_WRECK_COLS[0])
    }

    /// A point on the turret, `local` design pixels from the pivot in the
    /// turret's frame (`tank_art`: x to the right, y toward the tail), put
    /// in the world at the tank's facing - where a module's shot leaves.
    pub fn turret_point(&self, local: (f32, f32)) -> Position {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let (x, y) = (local.0 * self.scale, local.1 * self.scale);
        Position::new(self.position.x + x * cos - y * sin, self.position.y + x * sin + y * cos)
    }

    /// The main gun's muzzle on the gun line, `tank_muzzle_forward_offset`
    /// ahead of the pivot along `dir` (the facing a shot takes): where a
    /// laser beam and the flamethrower's cone are judged from, whichever
    /// module draws them.
    pub fn gun_line_muzzle(&self, dir: Vec2) -> Position {
        let ahead = tuning().tank_muzzle_forward_offset[self.row as usize] * self.scale;
        Position::new(self.position.x + dir.x * ahead, self.position.y + dir.y * ahead)
    }

    /// Whether this chassis's main gun is a twin (`tank_barrel_lateral_offset`).
    pub fn twin_gun(&self) -> bool {
        tuning().tank_barrel_lateral_offset[self.row as usize] > 0.0
    }

    /// The main gun fired: kick the recoil cells. A twin gun kicks its
    /// first barrel now and its second when the second shell leaves
    /// (`tank_twin_shot_delay_seconds` later); a single barrel kicks and
    /// then returns. `plasma` lights the plasma module's coils through it.
    pub fn kick(&mut self, plasma: bool) {
        let t = tuning();
        self.recoil_pose = 1;
        self.recoil_timer = t.tank_recoil_seconds;
        self.recoil_plasma = plasma;
        self.recoil_second_in = self.twin_gun().then_some(t.tank_twin_shot_delay_seconds);
    }

    /// The laser fired: its lens shows the beam for two recoil beats.
    pub fn kick_laser(&mut self) {
        self.laser_flash_timer = tuning().tank_recoil_seconds * 2.0;
    }

    /// Step the recoil cells `kick` set, and the laser's flash.
    pub fn tick_recoil(&mut self, dt: f32) {
        self.laser_flash_timer = (self.laser_flash_timer - dt).max(0.0);
        if let Some(left) = self.recoil_second_in {
            let left = left - dt;
            if left <= 0.0 {
                self.recoil_second_in = None;
                self.recoil_pose = 2;
                self.recoil_timer = tuning().tank_recoil_seconds;
                return;
            }
            self.recoil_second_in = Some(left);
        }
        if self.recoil_pose == 0 {
            return;
        }
        self.recoil_timer -= dt;
        if self.recoil_timer > 0.0 || self.recoil_second_in.is_some() {
            return;
        }
        if self.recoil_pose == 1 && !self.twin_gun() {
            // A single barrel on its way back.
            self.recoil_pose = 2;
            self.recoil_timer = tuning().tank_recoil_seconds;
        } else {
            self.recoil_pose = 0;
            self.recoil_plasma = false;
        }
    }

    /// Which weapon this tank's next trigger-pull actually fires: the
    /// first entry in `weapon_queue` that still has ammo (FIFO - the front
    /// weapon holds the trigger until it runs dry, then the next queued
    /// pickup takes over; see that field's doc comment), or the default
    /// shell cannon once every queued special is spent. Spent entries are
    /// *skipped* here rather than eagerly popped at every place ammo can
    /// hit zero (player dispatch, enemy dispatch, mid-burst) -
    /// `enqueue_weapon` prunes them on the next pickup instead. Purely
    /// picks the *tier* - whether that tier's own ammo is actually
    /// sufficient to fire *this instant* is still checked at each dispatch
    /// site in `Game::update` (a twin-barrel chassis needing 2 shells/bolts
    /// per shot can still be `ActiveWeapon::Shell`/`Plasma` while short of
    /// the 2 it needs, exactly as before this existed).
    pub fn active_weapon(&self) -> ActiveWeapon {
        self.weapon_queue
            .iter()
            .copied()
            .find(|&w| self.weapon_ammo(w) > 0)
            .unwrap_or(ActiveWeapon::Shell)
    }

    /// The ammo counter behind `weapon` - the one shared currency between
    /// the queue logic (`active_weapon`/`enqueue_weapon`) and the fire
    /// dispatch sites that actually decrement these fields.
    pub(crate) fn weapon_ammo(&self, weapon: ActiveWeapon) -> i32 {
        match weapon {
            ActiveWeapon::Laser => self.laser_charges,
            ActiveWeapon::Plasma => self.plasma_ammo,
            ActiveWeapon::Minigun => self.minigun_ammo,
            ActiveWeapon::Missiles => self.missile_ammo,
            // Whole seconds, rounded up: the last fraction still fires.
            ActiveWeapon::Flamethrower => self.flame_fuel.ceil().max(0.0) as i32,
            ActiveWeapon::Shell => self.shells_ammo,
        }
    }

    /// The flamethrower's slot readout: fuel in whole seconds, rounded
    /// up, 0 when dry.
    pub fn flame_fuel_seconds(&self) -> i32 {
        self.weapon_ammo(ActiveWeapon::Flamethrower)
    }

    /// Register a collected special-weapon pickup in `weapon_queue` (see
    /// that field's doc comment for the FIFO inventory rule). Call this
    /// *before* adding the pickup's ammo grant: it first prunes entries
    /// whose ammo has run dry - which must still read zero at that point,
    /// so a re-collected spent weapon re-enters at the *back* of the line
    /// instead of resurrecting in its old slot - then appends `weapon`
    /// unless a still-stocked batch of it is already queued (a repeat
    /// pickup is then just a top-up that keeps its place in line).
    pub fn enqueue_weapon(&mut self, weapon: ActiveWeapon) {
        let queue = std::mem::take(&mut self.weapon_queue);
        let kept: Vec<ActiveWeapon> = queue
            .into_iter()
            .filter(|&w| self.weapon_ammo(w) > 0)
            .collect();
        self.weapon_queue = kept;
        if !self.weapon_queue.contains(&weapon) {
            self.weapon_queue.push(weapon);
        }
    }

    /// Advance the minigun's barrel-cycle timer while a burst is active;
    /// hold it otherwise (see `minigun_cycle_timer`'s doc comment). Called
    /// once per frame for every tank alongside `tick_recharge`/`tick_wreck`
    /// in `Game::update`'s unified per-tank timer loop.
    pub fn tick_minigun_spin(&mut self, dt: f32) {
        if self.minigun_burst.is_some() {
            self.minigun_cycle_timer =
                (self.minigun_cycle_timer + dt) % (tuning().minigun_cycle_seconds * 3.0);
        }
    }

    /// Keep `missile_tubes_empty` in step with the launcher: a volley in
    /// flight shows the tubes it has emptied; after it, the tubes refill one
    /// by one as `fire_cooldown` runs down the reload. Called with the other
    /// per-tank timers, after `fire_cooldown` has ticked.
    pub fn tick_missile_pod(&mut self) {
        if self.missile_volley.is_some() || self.missile_tubes_empty == 0 {
            return;
        }
        let reload = tuning().missile_reload_seconds;
        let tubes = MISSILE_TUBE_OFFSETS.len() as f32;
        let empty = if reload > 0.0 { (self.fire_cooldown / reload * tubes).ceil().clamp(0.0, tubes) as u8 } else { 0 };
        self.missile_tubes_empty = self.missile_tubes_empty.min(empty);
    }

    /// Which of the minigun module's 3 "hot barrel" cells
    /// (`tank_modules.png`) to draw right now - see `minigun_cycle_timer`'s
    /// doc comment for why this is a discrete frame index, not a rotation
    /// angle.
    fn minigun_cycle_frame(&self) -> i32 {
        ((self.minigun_cycle_timer / tuning().minigun_cycle_seconds) as i32).clamp(0, 2)
    }

    /// Pull `ring_position` toward `position` as a damped spring: the ring
    /// accelerates toward the hull in proportion to how far behind it is
    /// (`tank_ring_spring_hz` sets how briskly) and bleeds off its own speed
    /// (`tank_ring_damping`, a damping ratio - under 1 lets it overshoot a
    /// touch as it settles). That gives the sleepy feel for free: a tank
    /// setting off leaves the ring behind for a beat, a cruising tank drags
    /// it at a steady offset that grows with speed (so a speed boost visibly
    /// stretches the trail), and a stopping tank has it glide in and settle.
    /// The trail is leashed to `tank_ring_max_trail_px` so the ring stays
    /// tucked under the hull no matter how fast it goes. Snaps outright when
    /// the hull is more than a body length away (spawn, teleport) so the
    /// ring never visibly flies across the map to catch up.
    pub fn ease_ring_position(&mut self, dt: f32) {
        let dx = self.position.x - self.ring_position.x;
        let dy = self.position.y - self.ring_position.y;
        let snap = self.size();
        if dx * dx + dy * dy > snap * snap {
            self.ring_position = self.position;
            self.ring_velocity = Vec2::new(0.0, 0.0);
            return;
        }
        let t = tuning();
        let omega = t.tank_ring_spring_hz * std::f32::consts::TAU;
        if omega <= 0.0 {
            self.ring_position = self.position;
            self.ring_velocity = Vec2::new(0.0, 0.0);
            return;
        }
        // Semi-implicit Euler: stable for the omega*dt this game runs at
        // (a few Hz at 60 fps), and cheap enough for every tank every frame.
        let damping = 2.0 * t.tank_ring_damping * omega;
        self.ring_velocity.x += (omega * omega * dx - damping * self.ring_velocity.x) * dt;
        self.ring_velocity.y += (omega * omega * dy - damping * self.ring_velocity.y) * dt;
        self.ring_position.x += self.ring_velocity.x * dt;
        self.ring_position.y += self.ring_velocity.y * dt;
        // Leash: never further than `tank_ring_max_trail_px` behind the hull.
        let dx = self.position.x - self.ring_position.x;
        let dy = self.position.y - self.ring_position.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let leash = t.tank_ring_max_trail_px;
        if dist > leash && dist > 0.0 {
            let pull = 1.0 - leash / dist;
            self.ring_position.x += dx * pull;
            self.ring_position.y += dy * pull;
        }
    }

    /// Small phase offset (seconds) derived from screen position so that several
    /// burning tanks don't animate their smoke/fire in perfect lockstep.
    pub fn anim_phase(&self) -> f32 {
        (self.position.x + self.position.y) * 0.01
    }

    /// Collision footprint side length: the visible hull, not the full sprite
    /// tile, so tanks can close the gap left by the sprite's transparent padding.
    /// A uniform-square approximation used by the AI's avoidance radius, the
    /// ground-decal rear-edge offset, and spawn-clearance checks - all of
    /// which only need an approximate footprint. The tank's actual physics
    /// collider is sized more precisely per row - see `hull_half_extents`.
    pub fn hull_size(&self) -> f32 {
        self.size() * TANK_HULL_FRACTION
    }

    /// Damage-box half-extents (x, y) for this tank's hull, in world px,
    /// oriented for the given facing - `along_x` true when facing Left/Right
    /// (width and height swap from the sprite's own "facing up" reference
    /// frame), matching `facing_along_x`. Distinct from `hull_size` above:
    /// this is the real per-row rectangle (TANK_HULL_BBOX_BY_ROW) - the full
    /// visible hull silhouette - that the projectile hit test checks. The
    /// solid *movement* collider is smaller: see `move_half_extents` below.
    pub fn hull_half_extents(&self, along_x: bool) -> (f32, f32) {
        let (w, h) = TANK_HULL_BBOX_BY_ROW[self.row as usize];
        let (w, h) = if along_x { (h, w) } else { (w, h) };
        (w * 0.5 * self.scale, h * 0.5 * self.scale)
    }

    /// Movement-collider half-extents (x, y): `hull_half_extents` scaled
    /// down by TANK_MOVE_BBOX_FRACTION (see that constant's doc comment for
    /// the damage-box-vs-movement-box split). This is the overall footprint
    /// the physics body actually blocks with (`Physics::spawn_tank`/
    /// `resize_collider` - which additionally round its corners, keeping
    /// this exact footprint; see `physics::tank_corner_radius`).
    pub fn move_half_extents(&self, along_x: bool) -> (f32, f32) {
        let (hx, hy) = self.hull_half_extents(along_x);
        (hx * TANK_MOVE_BBOX_FRACTION, hy * TANK_MOVE_BBOX_FRACTION)
    }

    /// True when `rotation` currently faces Left/Right rather than Up/Down -
    /// which axis is the hull's "long" one. `rotation` is always exactly one
    /// of `Dir::rotation()`'s four values (see `Tank::control`), so a plain
    /// `==` match is exact here, no epsilon needed.
    pub fn facing_along_x(&self) -> bool {
        self.rotation == Dir::Right.rotation() || self.rotation == Dir::Left.rotation()
    }

    /// This tank's hull collider footprint in world space right now -
    /// `position` as the center (the hull box is symmetric front-to-back
    /// and side-to-side, so it never needs an offset) and
    /// `hull_half_extents` oriented for the current facing - the hull box
    /// the projectile hit test checks (`simulation::hits::Terrain::sweep`).
    pub fn hull_bbox_world(&self) -> (Position, Position) {
        let (hw, hh) = self.hull_half_extents(self.facing_along_x());
        (self.position, Position::new(hw, hh))
    }

    /// World-space center and half-extents of this tank's turret+barrel
    /// bounding box (`TANK_TURRET_BBOX_BY_ROW`) at its current `rotation` -
    /// the second box the projectile hit test checks, and what the
    /// `hitboxes` debug overlay (`game.rs::draw_tank_boxes`) draws.
    /// Unlike `hull_half_extents`'s `along_x` swap (safe because
    /// the hull box is roughly centered on the tank), the turret+barrel box
    /// is off-center - the barrel extends it well past the tile center
    /// toward the front - so this needs a real per-direction rotation of
    /// both the offset and the extents, not just a width/height swap.
    pub fn turret_bbox_world(&self) -> (Position, Position) {
        let (x0, y0, x1, y1) = TANK_TURRET_BBOX_BY_ROW[self.row as usize];
        // Local "facing up" frame, origin at the tile center (16,16) -
        // same convention TANK_HULL_BBOX_BY_ROW's own values are measured
        // in.
        let half = TANK_FRAME_SIZE * 0.5;
        let local_cx = (x0 + x1 + 1.0) * 0.5 - half;
        let local_cy = (y0 + y1 + 1.0) * 0.5 - half;
        let local_hw = (x1 - x0 + 1.0) * 0.5;
        let local_hh = (y1 - y0 + 1.0) * 0.5;
        // Clockwise rotation of the local (x,y) offset by `rotation` - the
        // same mapping `Dir::vec` encodes (Up's local "forward", -y, must
        // land on each direction's own forward vector): identity at Up,
        // (x,y)->(-y,x) at Right, negate-both at Down, (x,y)->(y,-x) at
        // Left. Half-extents swap width/height exactly when the offset
        // rotation does (Right/Left), matching `hull_half_extents`'s own
        // along_x swap.
        let (ox, oy, hw, hh) = if self.rotation == Dir::Up.rotation() {
            (local_cx, local_cy, local_hw, local_hh)
        } else if self.rotation == Dir::Right.rotation() {
            (-local_cy, local_cx, local_hh, local_hw)
        } else if self.rotation == Dir::Down.rotation() {
            (-local_cx, -local_cy, local_hw, local_hh)
        } else {
            (local_cy, -local_cx, local_hh, local_hw)
        };
        let center = Position::new(
            self.position.x + ox * self.scale,
            self.position.y + oy * self.scale,
        );
        (center, Position::new(hw * self.scale, hh * self.scale))
    }

    /// A safe circular over-approximation of this tank's real (per-row)
    /// physics footprint at its current facing - the bounding-circle radius
    /// (hypot of both half-extents) of the *movement* collider
    /// (`move_half_extents`, the box the physics body actually blocks
    /// with - the corner rounding only ever cuts inside that box, so this
    /// still never under-estimates it), rather than the uniform
    /// `hull_size() * 0.5` approximation. Used anywhere collision math is
    /// circle-based (AI predictive avoidance - `ai.rs`'s
    /// `Mover.radius`/`AvoidCtx.radius`) so it never assumes a tank is
    /// smaller than the collider `Physics::resize_collider` actually gave
    /// it - a mismatch that let the AI drive tanks (titan/leviathan
    /// especially) into obstacles/other tanks it believed were clear,
    /// wedging them until an external impulse (a shell hit, or the player
    /// ramming through) dislodged them. Tracking the movement box rather
    /// than the full damage box also means the shrink
    /// (TANK_MOVE_BBOX_FRACTION) automatically buys the pathfinder tighter
    /// clearance margins - see `battlefield::max_tank_clearance_half_extent`.
    pub fn avoidance_radius(&self) -> f32 {
        let (hx, hy) = self.move_half_extents(self.facing_along_x());
        (hx * hx + hy * hy).sqrt()
    }

    /// A tank's mass, for both collision knockback and (via `Game::drive_tank`,
    /// which divides its accel/decel/turn-grip forces by this) how sluggish it
    /// is to speed up and how much it drifts through a turn. Proportional to
    /// `scale` squared - a genuine area normalization rather than an
    /// arbitrary number - scaled further by this tank's chassis class (see
    /// TANK_CHASSIS_MASS_FACTOR_BY_ROW, indexed by `row`): a `std`-class tank
    /// (assault/warden) has exactly the old flat mass every tank used to
    /// share, `narrow`/`compact` chassis are lighter (quicker to accelerate,
    /// less drift, shoved further in a ram), `long`/`wide` and especially the
    /// two `super_*` chassis are heavier (sluggish, more drift, shove lighter
    /// tanks further than they get shoved back).
    pub fn mass(&self) -> f32 {
        self.scale * self.scale * tuning().tank_mass_factor[self.row as usize]
    }

    /// Advance a live shield: run down the post-hit delay, then refill
    /// toward `shield_capacity`. Only a shield that still has charge
    /// recharges - once it shatters it is gone until another pickup, which
    /// is what keeps a rechargeable shield from being a permanent one.
    ///
    /// Called from `Game::tick_timers`, which runs before any hit phase in
    /// the frame, so a tank hit this frame has its delay reset *after* this
    /// frame's refill was considered and cannot regen on the same frame it
    /// was struck - no extra guard needed.
    pub fn tick_shield(&mut self, dt: f32) {
        if !self.is_shielded() {
            self.shield_recharge_delay = 0.0;
            return;
        }
        self.shield_recharge_delay = (self.shield_recharge_delay - dt).max(0.0);
        if self.shield_recharge_delay > 0.0 {
            return;
        }
        // Never *reduce* the pool: `shield_charge` already clamps its
        // report to 1, so a pool deliberately set past the knob (the
        // immortality fixtures, a tuning experiment) stays where it was put
        // rather than being pulled down to capacity on the first refill.
        let capacity = tuning().shield_capacity;
        if self.shield_hp >= capacity {
            return;
        }
        self.shield_hp = (self.shield_hp + tuning().shield_recharge_per_second * dt).min(capacity);
    }

    /// Recharge ammo over time toward MAX_SHELLS, one shell per interval.
    pub fn tick_recharge(&mut self, dt: f32) {
        if self.shells_ammo >= tuning().max_shells {
            self.recharge_timer = 0.0;
            return;
        }
        self.recharge_timer += dt;
        while self.recharge_timer >= tuning().shell_recharge_seconds && self.shells_ammo < tuning().max_shells
        {
            self.recharge_timer -= tuning().shell_recharge_seconds;
            self.shells_ammo += 1;
        }
    }

    /// Age a wreck so its fire burns for WRECK_BURN_SECONDS before going out. The
    /// timer only runs once the tank is a wreck; a live tank keeps it at zero.
    pub fn tick_wreck(&mut self, dt: f32) {
        if self.is_wreck() {
            // Cap the timer so it doesn't grow unbounded once the fire is out.
            self.wreck_timer = (self.wreck_timer + dt).min(tuning().wreck_burn_seconds);
        }
    }

    /// Reset `hit_flash_timer` to full - call whenever this tank takes
    /// damage (shell, ram, explosion splash, frog bite) so an enemy's health
    /// ring shows/refreshes for another `health_ring_hit_seconds`.
    pub fn mark_hit(&mut self) {
        self.hit_flash_timer = tuning().health_ring_hit_seconds;
    }

    /// `by` has just damaged this tank: a seat's hit on an enemy makes
    /// that seat the one its wreck is credited to (`last_hit_by`). An
    /// enemy's hit, or any hit on a seat, credits nobody.
    pub fn credit(&mut self, by: Owner) {
        if let Owner::Player(seat) = by
            && !self.owner.is_player()
        {
            self.last_hit_by = Some(seat);
        }
    }

    /// Decide this tank's rotation and commanded velocity for one frame.
    /// `move_dir` faces the hull that way and sets `velocity` to its
    /// damage-scaled speed along that axis (classic 4-direction, no momentum;
    /// see `effective_speed`). `face` turns the hull in place without moving
    /// (used when an AI stops to aim). `move_dir` takes precedence. Shared by
    /// the player and the AI so both move identically. Does not touch
    /// `position` - that's the physics body's job once `velocity` is handed
    /// to it; see `Game::drive_tank`.
    pub fn control(&mut self, move_dir: Option<Dir>, face: Option<Dir>) {
        if let Some(dir) = move_dir {
            self.rotation = dir.rotation();
            let step = dir.vec();
            let speed = self.effective_speed();
            self.velocity = Vec2::new(step.x * speed, step.y * speed);
        } else {
            self.velocity = Vec2::new(0.0, 0.0);
            if let Some(dir) = face {
                self.rotation = dir.rotation();
            }
        }
    }

    /// Chase `visual_rotation` toward `rotation` at TANK_VISUAL_TURN_SPEED_DEG
    /// degrees/second, the short way round, so the sprite visibly swings into
    /// a turn instead of popping to the new facing the instant `rotation`
    /// snaps (see the fields' own doc comments). Called once per frame for
    /// every tank from `Game::drive_tank`, right after `control` above sets
    /// this frame's `rotation`.
    pub fn ease_visual_rotation(&mut self, dt: f32) {
        let mut diff = (self.rotation - self.visual_rotation) % 360.0;
        if diff > 180.0 {
            diff -= 360.0;
        } else if diff < -180.0 {
            diff += 360.0;
        }
        let max_step = tuning().tank_visual_turn_speed_deg * dt;
        self.visual_rotation = (self.visual_rotation + diff.clamp(-max_step, max_step)) % 360.0;
    }

    /// Chase `turret_visual_rotation` toward `rotation` at
    /// TANK_TURRET_VISUAL_TURN_SPEED_DEG degrees/second, the short way round -
    /// same mechanism as `ease_visual_rotation`, just a separate angle and a
    /// faster rate, so the turret visibly gets to the new heading before the
    /// hull does. Called once per frame for every tank, alongside
    /// `ease_visual_rotation`.
    pub fn ease_turret_visual_rotation(&mut self, dt: f32) {
        let mut diff = (self.rotation - self.turret_visual_rotation) % 360.0;
        if diff > 180.0 {
            diff -= 360.0;
        } else if diff < -180.0 {
            diff += 360.0;
        }
        let max_step = tuning().tank_turret_visual_turn_speed_deg * dt;
        self.turret_visual_rotation =
            (self.turret_visual_rotation + diff.clamp(-max_step, max_step)) % 360.0;
    }
}

/// Rotation pivot for a tank sprite of the given on-screen `size`: not the
/// sprite's exact geometric center, but shifted TANK_PIVOT_REAR_FRACTION of
/// its width back toward the rear of the hull (the sprite's "down" edge in
/// its unrotated, facing-up orientation). `draw_texture_pro` rotates around
/// whichever point of the sprite this names while keeping that point pinned
/// to `tank.position`, so the visible hull ends up drawn shifted forward of
/// `position` by the same amount, at every facing - purely a draw-time
/// choice; nothing gameplay-relevant reads this.
fn draw_pivot(size: f32) -> Vec2 {
    Vec2::new(size / 2.0, size / 2.0 + size * TANK_PIVOT_REAR_FRACTION)
}

/// Which block of the sheet a tank draws from: 0 for an enemy, 1 to 4 for
/// the first four seats' recolours (blue, pink, silver-white, orange). A
/// seat past the fourth borrows player 1's block and is told apart by its
/// ring colour and its `P5`..`P8` label (docs/online-coop-prd.md §4.11).
fn sheet_block(player: Option<u8>) -> i32 {
    match player {
        None => 0,
        Some(seat) if (seat as i32) < TANK_TEAM_BLOCKS - 1 => seat as i32 + 1,
        Some(_) => 1,
    }
}

/// Source rectangles for a fixed representative tank - the Scout chassis's
/// pristine hull (col 0) and its turret at rest, in `player`'s team colour,
/// drawn in that order - used by the map editor's start-point palette icons
/// and placed-cell markers, which need one fixed tank rather than any
/// particular round's rolled chassis, in the colour that tank will actually
/// be. Each is the middle `TANK_FRAME_SIZE` square of its cell, so a marker
/// draws the art one to one on a map cell.
pub fn icon_source_recs(player: u8) -> [Rectangle; 2] {
    let row = sheet_block(Some(player)) * TANK_ROWS_PER_TEAM;
    let inset = (TANK_SPRITE_SIZE - TANK_FRAME_SIZE) / 2.0;
    [0, TANK_TURRET_COL].map(|col| {
        let cell = source_rec(row, col);
        Rectangle::new(cell.x + inset, cell.y + inset, TANK_FRAME_SIZE, TANK_FRAME_SIZE)
    })
}

/// Source rectangles for `kind`'s pristine hull and its turret at rest in
/// `player`'s team colour, drawn one over the other into the same 32 px
/// box - the builder's MAP panel showing the chassis a map pins for a seat.
/// Each is a `TANK_FRAME_SIZE` window of its cell, two pixels above the
/// middle one, which holds every chassis whole: the longest barrels reach
/// two pixels past the frame the pivot centres.
pub fn chassis_icon_source_recs(kind: TankKind, player: u8) -> [Rectangle; 2] {
    let row = kind.row() + sheet_block(Some(player)) * TANK_ROWS_PER_TEAM;
    let inset = (TANK_SPRITE_SIZE - TANK_FRAME_SIZE) / 2.0;
    [0, TANK_TURRET_COL].map(|col| {
        let cell = source_rec(row, col);
        Rectangle::new(cell.x + inset, cell.y + inset - CHASSIS_ICON_RAISE, TANK_FRAME_SIZE, TANK_FRAME_SIZE)
    })
}

/// How far `chassis_icon_source_recs` raises its window over the middle
/// frame, in sheet pixels.
const CHASSIS_ICON_RAISE: f32 = 2.0;

/// Source rectangle for the cell at (row, col) of the tank sheet (and of
/// its light layer, which has the same layout).
fn source_rec(row: i32, col: i32) -> Rectangle {
    Rectangle::new(col as f32 * TANK_SPRITE_SIZE, row as f32 * TANK_SPRITE_SIZE, TANK_SPRITE_SIZE, TANK_SPRITE_SIZE)
}

/// Source rectangle for a weapon module's cell: a row per chassis.
fn module_rec(chassis_row: i32, col: i32) -> Rectangle {
    source_rec(chassis_row, col)
}

/// Where the turret layer is drawn and at what angle: on the ring at the
/// turret's eased aim, or - on a blown wreck - thrown clear, lying beside
/// the hull on its back.
fn turret_placement(tank: &Tank) -> (Position, f32) {
    if tank.turret_thrown() {
        let r = tank.visual_rotation.to_radians();
        let (dx, dy) = (12.0 * tank.scale, 9.0 * tank.scale);
        let off = Vec2::new(dx * r.cos() - dy * r.sin(), dx * r.sin() + dy * r.cos());
        (tank.position + off, tank.visual_rotation + 150.0)
    } else {
        (tank.position, tank.turret_visual_rotation)
    }
}

/// One layer of a tank's sprite: (sheet, source, centre, rotation).
type Layer = (Sheet, Rectangle, Position, f32);

/// The hull layer of a tank's sprite. `glow` picks the light layer's
/// sheet; `time` paces, on the light layer, a damaged tank's sparks and
/// warning lamp (which the track frame alone would freeze on a tank
/// standing still).
fn hull_layer(tank: &Tank, time: f32, glow: bool) -> Layer {
    let sheet = if glow { Sheet::TankGlow } else { Sheet::Tanks };
    let mut col = tank.hull_col();
    if glow && !tank.is_wreck() && tank.damage_tier() >= 2 {
        let frame = ((time * 8.0 + tank.anim_phase()) as i32).rem_euclid(TANK_TRACK_FRAMES);
        col = tank.damage_tier() * TANK_TRACK_FRAMES + frame;
    }
    (sheet, source_rec(tank.sheet_row(), col), tank.position, tank.visual_rotation)
}

/// The layers over the hull, in draw order: the turret, then the weapon
/// modules it carries (`time` paces the flamethrower's pilot). `glow`
/// picks the light layer's sheets.
fn turret_layers(tank: &Tank, time: f32, glow: bool) -> Vec<Layer> {
    let (tanks, modules) = if glow { (Sheet::TankGlow, Sheet::TankModulesGlow) } else { (Sheet::Tanks, Sheet::TankModules) };
    let (at, rot) = turret_placement(tank);
    let mut out = vec![(tanks, source_rec(tank.sheet_row(), tank.turret_col()), at, rot)];
    for col in module_cols(tank, time).into_iter().flatten() {
        out.push((modules, module_rec(tank.row, col), tank.position, tank.turret_visual_rotation));
    }
    out
}

/// Every layer of a tank's sprite in draw order - hull, turret, the weapon
/// modules it carries.
fn layers(tank: &Tank, time: f32, glow: bool) -> Vec<Layer> {
    let mut out = vec![hull_layer(tank, time, glow)];
    out.extend(turret_layers(tank, time, glow));
    out
}

/// Blit `layers` at their 40 px cells round the shared pivot, in `tint`.
fn blit_layers(c: &mut impl Canvas, tank: &Tank, layers: &[Layer], tint: Color) {
    let size = tank.sprite_size();
    let origin = draw_pivot(size);
    for &(sheet, src, at, rot) in layers {
        c.blit(sheet, src, Rectangle::new(at.x, at.y, size, size), origin, rot, tint);
    }
}

/// Which weapon-module cells a tank shows: one per special weapon it holds,
/// each in the state the game already tracks - the minigun's hot barrel
/// while a burst fires (`minigun_cycle_frame`), the launcher's empty tubes
/// (`missile_tubes_empty`), plasma and laser idle / armed (the live weapon)
/// / firing, the flamethrower's pilot flickering or its jet while held. A
/// module is hardware, not a firing-mode indicator: it shows whenever the
/// weapon is carried, and a wreck carries none.
fn module_cols(tank: &Tank, time: f32) -> [Option<i32>; 5] {
    if tank.is_wreck() {
        return [None; 5];
    }
    let live = tank.active_weapon();
    let minigun = (tank.minigun_ammo > 0 || tank.minigun_burst.is_some()).then(|| {
        TANK_MODULE_MINIGUN_COL + if tank.minigun_burst.is_some() { 1 + tank.minigun_cycle_frame() } else { 0 }
    });
    let missiles = (tank.missile_ammo > 0 || tank.missile_volley.is_some())
        .then(|| TANK_MODULE_MISSILES_COL + (tank.missile_tubes_empty as i32).clamp(0, MISSILE_TUBE_OFFSETS.len() as i32));
    let plasma = (tank.plasma_ammo > 0 || (tank.recoil_plasma && tank.recoil_pose > 0)).then(|| {
        TANK_MODULE_PLASMA_COL
            + if tank.recoil_plasma && tank.recoil_pose > 0 {
                2
            } else if live == ActiveWeapon::Plasma {
                1
            } else {
                0
            }
    });
    let laser = (tank.laser_charges > 0 || tank.laser_flash_timer > 0.0).then(|| {
        TANK_MODULE_LASER_COL
            + if tank.laser_flash_timer > 0.0 {
                2
            } else if live == ActiveWeapon::Laser {
                1
            } else {
                0
            }
    });
    let flame = (tank.flame_fuel > 0.0).then(|| {
        TANK_MODULE_FLAME_COL
            + if tank.flame_held {
                2 + ((time * 14.0) as i32).rem_euclid(2)
            } else {
                ((time * 6.0 + tank.anim_phase()) as i32).rem_euclid(2)
            }
    });
    [minigun, missiles, plasma, laser, flame]
}

/// Draw a tank: hull, turret and the weapon modules it carries, each at its
/// own eased angle (`visual_rotation` for the hull, `turret_visual_rotation`
/// for the turret and its modules) - the turret chases the commanded
/// `rotation` faster than the hull, so it visibly leads a turn. Every layer
/// is a 40 px cell (`sprite_size`) drawn round the shared pivot.
pub fn draw_tank(c: &mut impl Canvas, tank: &Tank, time: f32) {
    draw_tank_hull(c, tank, time, tank.tint());
    draw_tank_turret(c, tank, time, tank.tint());
}

/// The hull layer of `draw_tank` alone, in `tint` - a hit's flash draws
/// both halves again in light (`render/game.rs`).
pub fn draw_tank_hull(c: &mut impl Canvas, tank: &Tank, time: f32, tint: Color) {
    blit_layers(c, tank, &[hull_layer(tank, time, false)], tint);
}

/// The turret layer of `draw_tank` and the weapon modules on it, in `tint`.
pub fn draw_tank_turret(c: &mut impl Canvas, tank: &Tank, time: f32, tint: Color) {
    blit_layers(c, tank, &turret_layers(tank, time, false), tint);
}

/// The tank's light layer - lamps, accent strips, the sensor eye, sparks,
/// embers, the modules' lenses and hot barrels (`scifi_tanks_glow.png`,
/// `tank_modules_glow.png`) - at `strength` 0..1, the dark sky's own light
/// level. Drawn additively over the field the sky has multiplied down, so
/// every lamp shines at night; the paint already carries the same pixels
/// at their daylight colours, which is why a clear day draws none of this.
/// A burning wreck's embers breathe.
pub fn draw_tank_glow(c: &mut impl Canvas, tank: &Tank, time: f32, strength: f32) {
    if strength <= 0.0 {
        return;
    }
    let mut k = strength.clamp(0.0, 1.0) * tank.alpha();
    if tank.is_wreck() {
        let p = tank.anim_phase() * 7.0;
        k *= 0.72 + 0.28 * (time * 11.0 + p).sin() * (time * 4.3 + p * 1.7).sin();
    }
    let tint = Color::new(255, 255, 255, (255.0 * k).round().clamp(0.0, 255.0) as u8);
    blit_layers(c, tank, &layers(tank, time, true), tint);
}

/// A tank coated in ooze from a bio slush tower
/// (docs/defence-towers-prd.md section 12): hull and turret again, washed
/// in the ooze's colour, with a few blobs stuck on; the wash fades out over
/// the coat's last half second.
pub fn draw_tank_slime(c: &mut impl Canvas, tank: &Tank, time: f32) {
    if !tank.is_slimed() || tank.is_wreck() {
        return;
    }
    let fade = (tank.slime_timer / 0.5).clamp(0.0, 1.0);
    let size = tank.sprite_size();
    let dest = Rectangle::new(tank.position.x, tank.position.y, size, size);
    let origin = draw_pivot(size);
    let wash = |col: Color, a: f32| Color::new(col.r, col.g, col.b, (a * fade) as u8);
    c.blit(Sheet::Tanks, source_rec(tank.sheet_row(), tank.hull_col()), dest, origin, tank.visual_rotation, wash(crate::tower::OOZE_MD, 120.0));
    c.blit(Sheet::Tanks, source_rec(tank.sheet_row(), tank.turret_col()), dest, origin, tank.turret_visual_rotation, wash(crate::tower::OOZE_LT, 90.0));
    let (center, half) = tank.hull_bbox_world();
    let seed = crate::blast::seed_at(Position::new(tank.owner_slot() as f32 * 32.0, 0.0), 61);
    for k in 0..7u32 {
        let h = seed.rotate_left(k * 5) ^ k.wrapping_mul(0x9e37_79b9);
        let dx = ((h % 1000) as f32 / 1000.0 - 0.5) * 2.0 * half.x * 0.8;
        // Drips run down the hull on a slow clock.
        let run = ((time * 0.7 + (h >> 12) as f32 / 1000.0).fract() * 6.0).floor() * 2.0 * (k % 2) as f32;
        let dy = (((h >> 20) % 1000) as f32 / 1000.0 - 0.5) * 2.0 * half.y * 0.8 + run;
        let x = ((center.x + dx) / 2.0).round() as i32 * 2;
        let y = ((center.y + dy) / 2.0).round() as i32 * 2;
        let col = if k % 3 == 0 { crate::tower::OOZE_LT } else { crate::tower::OOZE_MD };
        c.fill_rect(x, y, 2 + 2 * (k % 2) as i32, 2, wash(col, 230.0));
    }
}

// Puny Palette entries (tools/punypalette.py) the health gauge draws with -
// literals rather than a sheet sample, like `fx.rs`'s particle tints, since
// a ring is drawn, not blitted.
const GOLD_BRIGHT: Color = Color::new(0xEE, 0xA3, 0x43, 255);
const RED_BRIGHT: Color = Color::new(0xFF, 0x42, 0x1A, 255);
const RED_MD: Color = Color::new(0xE4, 0x42, 0x19, 255);
const RED_DEEP: Color = Color::new(0x9C, 0x35, 0x27, 255);
const RED_DK: Color = Color::new(0x81, 0x2F, 0x27, 255);
const RED_DARKEST: Color = Color::new(0x4A, 0x22, 0x21, 255);
pub(crate) const BLACK: Color = Color::new(0x25, 0x25, 0x25, 255);
/// One identity colour per seat (docs/player-indicator-improvements.md,
/// docs/PALETTE.md): the base step of the team ramp each of the sheet's
/// four player blocks is painted in - seat 0 sky blue, seat 1 hot pink,
/// seat 2 silver-white, seat 3 orange - then four more for the seats a room
/// adds, which borrow player 1's block. They stay off the Puny Palette: an
/// identity has to be loud against the terrain. Orange sits closest to the
/// enemies' warm hulls and to the gold and red the health ramp steps
/// through, so seat 3 leans on its bright light step and its `P4` label. The hull, the ground ring, the HUD readouts and button, the
/// editor's start markers and the round-start locate cue all draw from this
/// one table, so every surface reads as one identity.
pub const TEAM_COLORS: [Color; crate::MAX_SEATS] = [
    Color::new(0x4D, 0x9B, 0xE6, 255), // P1 sky blue
    Color::new(0xF0, 0x4F, 0x78, 255), // P2 hot pink
    Color::new(0xD3, 0xDA, 0xE3, 255), // P3 silver-white
    Color::new(0xFB, 0x6B, 0x1D, 255), // P4 orange
    Color::new(0x4D, 0x65, 0xB4, 255), // P5 royal blue
    Color::new(0xC3, 0x24, 0x54, 255), // P6 crimson
    Color::new(0x90, 0x5E, 0xA9, 255), // P7 grape
    Color::new(0x0B, 0x8A, 0x8F, 255), // P8 deep teal
];
/// The light step of each team ramp: the sheet's accent for the four seats
/// it draws, the colour their marker lights glow in at night, and every
/// seat's ring colour at full health, so a healthy ring reads brighter than
/// the hull.
const TEAM_LIGHT: [Color; crate::MAX_SEATS] = [
    Color::new(0x8F, 0xD3, 0xFF, 255),
    Color::new(0xED, 0x80, 0x99, 255),
    Color::new(0xF4, 0xF8, 0xFF, 255),
    Color::new(0xFF, 0xA8, 0x3A, 255),
    Color::new(0x7C, 0x92, 0xD8, 255),
    Color::new(0xE8, 0x63, 0x7F, 255),
    Color::new(0xC3, 0x98, 0xD6, 255),
    Color::new(0x4F, 0xC0, 0xC4, 255),
];

/// `TEAM_COLORS`/`TEAM_LIGHT` for a seat, wrapping past `MAX_SEATS` rather
/// than panicking - a seat index only ever comes from an owner slot, but a
/// wire message is not this build's to trust.
pub fn team_color(seat: u8) -> Color {
    TEAM_COLORS[seat as usize % TEAM_COLORS.len()]
}

/// Where a health gauge's filled arc starts, in raylib degrees: 12 o'clock.
/// raylib measures from +x and, on a y-down screen, increasing angles run
/// clockwise, so -90 is straight up and `start + sweep` walks clockwise
/// round the ring.
const HEALTH_RING_START_DEG: f32 = -90.0;

/// The four-colour ramp a health gauge steps through, one colour per quarter
/// of health (`health_ring_step`): brightest above three quarters, darkest at
/// a quarter or less. Stepped rather than blended so the ring reads as drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HealthRamp {
    /// White, gold, bright red, deep red: tanks and the player's frog.
    White,
    /// Bright red down to the darkest red: the enemy frog, whose ring is red
    /// at any health so its side still reads.
    Red,
    /// A seat: its own light and base team colours while healthy, then the
    /// same gold and bright red as `White` for the shared danger steps.
    Team(u8),
}

impl HealthRamp {
    const WHITE_STEPS: [Color; 4] = [Color::WHITE, GOLD_BRIGHT, RED_BRIGHT, RED_DEEP];
    const RED_STEPS: [Color; 4] = [RED_BRIGHT, RED_DEEP, RED_DK, RED_DARKEST];
    /// The ramp for the seat in `index`.
    pub fn player(index: u8) -> Self {
        Self::Team(index % crate::MAX_SEATS as u8)
    }

    /// The step colour for `frac` remaining health.
    pub fn color(self, frac: f32) -> Color {
        let steps = match self {
            Self::White => Self::WHITE_STEPS,
            Self::Red => Self::RED_STEPS,
            Self::Team(i) => {
                let i = i as usize % TEAM_COLORS.len();
                [TEAM_LIGHT[i], TEAM_COLORS[i], GOLD_BRIGHT, RED_BRIGHT]
            }
        };
        steps[health_ring_step(frac)]
    }

    /// The ramp's marker colour - what the missing part of a ring that stays
    /// a full circle is drawn in, dimmed: white, the enemy frog's red, or a
    /// player's team colour.
    pub fn base(self) -> Color {
        match self {
            Self::White => Color::WHITE,
            Self::Red => RED_MD,
            Self::Team(i) => team_color(i),
        }
    }
}

/// How a ground ring is coloured - the one thing that differs between the
/// rainbow shield ring, a plain marker and a health gauge. Size, thickness,
/// breathing, translucency and placement are all shared in
/// `draw_ground_ring_at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RingStyle {
    /// Six 60-degree arcs one hue step apart, all cycling through the
    /// rainbow starting from `base_hue` (degrees), with the arcs rotating
    /// along with it so the bands visibly travel around the ring; the
    /// inner disc takes the leading hue. A gauge like `Gauge`: `charge` is
    /// the shield time left (0..=1) and the bands fill that fraction of the
    /// circle from 12 o'clock clockwise, the rest drawn in `base` (its own
    /// alpha is its opacity), so the shield can be seen running down.
    Rainbow { base_hue: f32, charge: f32, base: Color },
    /// One flat colour for both the ring and its inner disc. The colour's
    /// own alpha is the band's opacity (the inner disc takes a quarter of
    /// it) - the shield's translucency is far too faint for a plain colour
    /// to read as itself over grass, so a solid ring chooses its own.
    Solid(Color),
    /// A health gauge: the band is a donut whose filled arc starts at 12
    /// o'clock and runs clockwise for `frac` of the circle, in `ramp`'s step
    /// colour at `player_ring_opacity`; the rest of the circle is drawn in
    /// `base`, whose own alpha is its opacity, so a marker ring stays a full
    /// circle while reading as a gauge (and an enemy's gap can be a dark
    /// band instead). The inner disc takes the step colour at a quarter of
    /// the band's opacity, as `Solid` does.
    Gauge { frac: f32, ramp: HealthRamp, base: Color },
}

/// Draw one translucent ground ring under a tank: a band of radius
/// `Tank::size() * shield_glow_radius_factor` (breathing gently around it
/// for the rainbow style, fixed for a solid one) plus a faint disc inside,
/// coloured per `style` and scaled by `fade` (0..=1).
/// Called before `draw_tank_shadow`, so it is a ground decal under the whole
/// tank - the sprite stays crisp and only the part reaching past the hull
/// shows. Centered on `Tank::ring_position`, the eased follower of the hull,
/// not the rear-shifted `draw_pivot`. `draw_tank_shield`, `draw_player_ring`
/// and `draw_enemy_ring` all come through here, so the shield ring and the
/// health gauges read as the same object in different colours.
pub fn draw_ground_ring(c: &mut impl Canvas, tank: &Tank, time: f32, style: RingStyle, fade: f32) {
    draw_ground_ring_scaled(c, tank.ring_position, tank.size(), tank.anim_phase(), time, style, fade, ring_scale(tank));
}

/// How much larger than the shared ring every tank's rings are drawn,
/// `tank_ring_radius_scale` - the same for players and enemies, so a hit
/// enemy's gauge sits at the radius the player's marker does.
fn ring_scale(_tank: &Tank) -> f32 {
    tuning().tank_ring_radius_scale
}

/// `draw_ground_ring` for anything that is not a tank - a frog's side
/// marker (`frog::draw_frog_ring`): the same ring at `center`, sized from
/// `size` (the sprite's on-screen side length) exactly as a tank's is from
/// `Tank::size`, with `phase` offsetting the rainbow's breathing.
pub fn draw_ground_ring_at(
    c: &mut impl Canvas,
    center: Position,
    size: f32,
    phase: f32,
    time: f32,
    style: RingStyle,
    fade: f32,
) {
    draw_ground_ring_scaled(c, center, size, phase, time, style, fade, 1.0);
}

/// `draw_ground_ring_at` with the radius scaled by `radius_scale` at the
/// *unscaled* band thickness, so a larger ring is a wider halo, not a
/// fatter one (every tank's rings, `tank_ring_radius_scale`).
#[allow(clippy::too_many_arguments)]
pub fn draw_ground_ring_scaled(
    c: &mut impl Canvas,
    center: Position,
    size: f32,
    phase: f32,
    time: f32,
    style: RingStyle,
    fade: f32,
    radius_scale: f32,
) {
    if fade <= 0.0 {
        return;
    }
    // Only the rainbow breathes (radius and alpha ride a slow sine); a solid
    // ring or a gauge is deliberately static - a plain, steady marker - so
    // it uses the sine's midpoint as fixed values and reads the same
    // size/opacity on average as the shield ring.
    let pulse = match style {
        RingStyle::Rainbow { .. } => ((time + phase) * std::f32::consts::TAU * 1.5).sin() * 0.5 + 0.5,
        RingStyle::Solid(_) | RingStyle::Gauge { .. } => 0.5,
    };
    let base_radius = size * tuning().shield_glow_radius_factor * (0.94 + 0.06 * pulse);
    let radius = base_radius * radius_scale;
    let thickness = base_radius * 0.22;
    let with_alpha = |c: Color, alpha: f32| Color::new(c.r, c.g, c.b, (alpha * fade).clamp(0.0, 255.0) as u8);
    let (disc_alpha, band_alpha) = match style {
        RingStyle::Rainbow { .. } => (22.0 + 10.0 * pulse, 95.0 + 30.0 * pulse),
        RingStyle::Solid(color) => (color.a as f32 * 0.25, color.a as f32),
        RingStyle::Gauge { .. } => {
            let band = tuning().player_ring_opacity * 255.0;
            (band * 0.25, band)
        }
    };

    match style {
        RingStyle::Rainbow { base_hue, charge, base } => {
            let fill = Color::color_from_hsv(base_hue, 0.6, 1.0);
            c.disc(center, radius - thickness, with_alpha(fill, disc_alpha));
            let sweep = health_ring_sweep(charge);
            if sweep < 360.0 {
                let gap = with_alpha(base, base.a as f32);
                let segments = health_ring_segments(360.0 - sweep);
                let end = HEALTH_RING_START_DEG + sweep;
                c.ring(center, radius - thickness, radius, end, HEALTH_RING_START_DEG + 360.0, segments, gap);
            }
            const ARCS: i32 = 6;
            let step = 360.0 / ARCS as f32;
            for i in 0..ARCS {
                let hue = (base_hue + i as f32 * step).rem_euclid(360.0);
                let color = with_alpha(Color::color_from_hsv(hue, 0.85, 1.0), band_alpha);
                // Where this band starts, measured from 12 o'clock clockwise;
                // the bands travel round the ring with the hue.
                let from = (i as f32 * step - base_hue - HEALTH_RING_START_DEG).rem_euclid(360.0);
                for (a, b) in arc_pieces(from, step, sweep) {
                    if b > a {
                        let segments = health_ring_segments(b - a);
                        let (start, end) = (HEALTH_RING_START_DEG + a, HEALTH_RING_START_DEG + b);
                        c.ring(center, radius - thickness, radius, start, end, segments, color);
                    }
                }
            }
        }
        RingStyle::Solid(color) => {
            c.disc(center, radius - thickness, with_alpha(color, disc_alpha));
            c.ring(center, radius - thickness, radius, 0.0, 360.0, 48, with_alpha(color, band_alpha));
        }
        RingStyle::Gauge { frac, ramp, base } => {
            let fill = ramp.color(frac);
            c.disc(center, radius - thickness, with_alpha(fill, disc_alpha));
            let sweep = health_ring_sweep(frac);
            let end = HEALTH_RING_START_DEG + sweep;
            // The missing part first and the health arc over it, so the
            // arc's end cap wins at the seam. raylib draws nothing for a
            // zero-width arc but mirrors a reversed one, hence the guards.
            if sweep < 360.0 {
                let gap = with_alpha(base, base.a as f32);
                let segments = health_ring_segments(360.0 - sweep);
                c.ring(center, radius - thickness, radius, end, HEALTH_RING_START_DEG + 360.0, segments, gap);
            }
            if sweep > 0.0 {
                let arc = with_alpha(fill, band_alpha);
                let segments = health_ring_segments(sweep);
                c.ring(center, radius - thickness, radius, HEALTH_RING_START_DEG, end, segments, arc);
            }
        }
    }
}

/// How far into its final fade-out a tank's shield is: 1 while it holds more
/// than `shield_glow_fade_fraction` of its charge, falling to 0 as the last
/// of it is spent, 0 when there is no shield at all. Drives the shield ring's
/// opacity and, inverted, the health ring's
/// (`player_health_ring_visibility`, `enemy_health_ring_visibility`), so the
/// two cross-fade instead of stacking - a shield about to shatter shows the
/// hull it was hiding.
fn shield_visibility(tank: &Tank) -> f32 {
    if !tank.is_shielded() {
        return 0.0;
    }
    let fade_fraction = tuning().shield_glow_fade_fraction;
    if fade_fraction > 0.0 { (tank.shield_charge() / fade_fraction).min(1.0) } else { 1.0 }
}

/// The visible pieces of a ring band that starts `from` degrees past 12
/// o'clock and runs `len` degrees clockwise, inside a gauge window of
/// `sweep` degrees from 12 o'clock: the band may wrap past 12 o'clock, so
/// the second piece is the wrapped remainder. Pieces with `b <= a` are
/// empty. Degrees are measured from 12 o'clock, not raylib's +x.
pub fn arc_pieces(from: f32, len: f32, sweep: f32) -> [(f32, f32); 2] {
    let end = from + len;
    [(from, end.min(360.0).min(sweep)), (0.0, (end - 360.0).min(sweep))]
}

/// Draw the rainbow shield ring while `Tank::shield_hp` lasts: a
/// `RingStyle::Rainbow` ground ring whose hue cycles at `shield_glow_hue_hz`
/// (offset by `anim_phase` so neighbouring tanks don't cycle in lockstep),
/// its bands covering the fraction of shield time left
/// (`Tank::shield_charge`) from 12 o'clock clockwise and the rest drawn as
/// the tank's health ring draws its missing part - dimmed white for the
/// player, a dark band for an enemy - so it visibly runs down; fading out
/// below `shield_glow_fade_fraction` of its charge.
pub fn draw_tank_shield(c: &mut impl Canvas, tank: &Tank, time: f32) {
    if tank.is_wreck() {
        return;
    }
    let base_hue = (time * tuning().shield_glow_hue_hz * 360.0 + tank.anim_phase() * 360.0).rem_euclid(360.0);
    let base = match tank.owner() {
        Owner::Player(i) => with_opacity(team_color(i), tuning().player_ring_opacity * tuning().health_ring_base_opacity),
        Owner::Enemy(_) | Owner::Tower { .. } => with_opacity(BLACK, tuning().health_ring_gap_opacity),
    };
    let style = RingStyle::Rainbow { base_hue, charge: tank.shield_charge(), base };
    draw_ground_ring(c, tank, time, style, shield_visibility(tank));
}

/// Which ramp step `frac` (remaining health, 0..=1) falls in: 0 above three
/// quarters, 1 above half, 2 above a quarter, 3 at or below.
pub fn health_ring_step(frac: f32) -> usize {
    if frac > 0.75 {
        0
    } else if frac > 0.5 {
        1
    } else if frac > 0.25 {
        2
    } else {
        3
    }
}

/// Degrees of ring the filled arc covers for `frac` remaining health.
pub fn health_ring_sweep(frac: f32) -> f32 {
    360.0 * frac.clamp(0.0, 1.0)
}

/// Segments for a `sweep`-degree arc at the full ring's density (48 for the
/// whole circle, 7.5 degrees each), at least one. Never below raylib's own
/// one-per-quadrant floor, so the count is never overridden.
fn health_ring_segments(sweep: f32) -> i32 {
    ((48.0 * sweep / 360.0).ceil() as i32).max(1)
}

/// `color` at `opacity` (0..=1).
pub fn with_opacity(color: Color, opacity: f32) -> Color {
    Color::new(color.r, color.g, color.b, (opacity * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// How much of the hit window `Tank::mark_hit` opened is still showing: 1
/// while more than `health_ring_hit_fade_seconds` of it remain, then down
/// to 0 as the window closes.
fn hit_ring_visibility(tank: &Tank) -> f32 {
    if tank.hit_flash_timer <= 0.0 {
        return 0.0;
    }
    let fade = tuning().health_ring_hit_fade_seconds;
    if fade > 0.0 { (tank.hit_flash_timer / fade).min(1.0) } else { 1.0 }
}

/// Opacity factor (0..=1) of the player's health ring: always on, yielding
/// to the shield ring as that fades in and out; 0 for a wreck.
pub fn player_health_ring_visibility(tank: &Tank) -> f32 {
    if tank.is_wreck() { 0.0 } else { 1.0 - shield_visibility(tank) }
}

/// Opacity factor (0..=1) of an enemy tank's health ring: on for
/// `health_ring_hit_seconds` after a hit, fading over the last
/// `health_ring_hit_fade_seconds`, or for as long as its remaining health is
/// at or below `enemy_health_ring_below`; yields to the shield ring like the
/// player's; 0 for a wreck.
pub fn enemy_health_ring_visibility(tank: &Tank) -> f32 {
    if tank.is_wreck() {
        return 0.0;
    }
    let low = if tank.health_fraction() <= tuning().enemy_health_ring_below { 1.0 } else { 0.0 };
    hit_ring_visibility(tank).max(low) * (1.0 - shield_visibility(tank))
}

/// Draw a player's marker ring as its health gauge: the same ground ring
/// as the shield (same thickness, `tank_ring_radius_scale` wider, minus
/// the breathing), the remaining-health arc in the player's team ramp
/// (`HealthRamp::player`) at `player_ring_opacity` and the rest of the
/// circle in the team colour dimmed to `health_ring_base_opacity` of that,
/// so each human tank always has a steady full halo in its own colour that
/// also reads its health. Yields to the shield ring while one is up and
/// disappears with the wreck (`player_health_ring_visibility`). An enemy
/// handed to this draws as player 1.
pub fn draw_player_ring(c: &mut impl Canvas, tank: &Tank, time: f32) {
    let ramp = HealthRamp::player(tank.player_index().unwrap_or(0));
    let base = with_opacity(ramp.base(), tuning().player_ring_opacity * tuning().health_ring_base_opacity);
    let style = RingStyle::Gauge { frac: tank.health_fraction(), ramp, base };
    draw_ground_ring(c, tank, time, style, player_health_ring_visibility(tank));
}

/// Draw an enemy's health ring: the same gauge in the enemy frog's all-red
/// ramp, its missing part a dark band at `health_ring_gap_opacity`, so a
/// just-hit enemy at full health reads hostile rather than borrowing a
/// player's colour. Shown per `enemy_health_ring_visibility` - after a
/// hit, or for good once low.
pub fn draw_enemy_ring(c: &mut impl Canvas, tank: &Tank, time: f32) {
    let base = with_opacity(BLACK, tuning().health_ring_gap_opacity);
    let style = RingStyle::Gauge { frac: tank.health_fraction(), ramp: HealthRamp::Red, base };
    draw_ground_ring(c, tank, time, style, enemy_health_ring_visibility(tank));
}

/// The round-start locate cue's ripple: for `player_locate_seconds` after
/// the round became playable (`elapsed`, which is `Game::time` - it does
/// not run behind the mission banner) a team-coloured ring swells from the
/// player's own ring out to 1.6x its radius and fades as it goes,
/// `player_locate_pulse_hz` times a second. Drawn under the hull like the
/// other rings; `render::tank::draw_player_label` is the cue's other half. Nothing for a
/// wreck, an enemy, or once the window has passed.
pub fn draw_player_locate(c: &mut impl Canvas, tank: &Tank, time: f32, elapsed: f32) {
    let Some(index) = tank.player_index() else { return };
    if tank.is_wreck() || !player_locate_active(elapsed) {
        return;
    }
    let phase = (elapsed * tuning().player_locate_pulse_hz).fract();
    let color = with_opacity(team_color(index), (1.0 - phase) * tuning().player_ring_opacity);
    let scale = ring_scale(tank) * (1.0 + 0.6 * phase);
    draw_ground_ring_scaled(c, tank.ring_position, tank.size(), tank.anim_phase(), time, RingStyle::Solid(color), 1.0, scale);
}

/// Whether the locate cue is still showing `elapsed` seconds into play.
pub fn player_locate_active(elapsed: f32) -> bool {
    elapsed < tuning().player_locate_seconds
}

/// Draw this tank's drop shadow: every layer `draw_tank` draws (each at its
/// own eased angle), offset toward a fixed screen-space direction and
/// tinted flat black - see docs/sprite-shadows-design.md. Must be called
/// *before* `draw_tank` so the real sprite draws on top of its own shadow.
/// A burnt-out hulk is still a solid object sitting on the ground, so a
/// wreck casts one too.
pub fn draw_tank_shadow(c: &mut impl Canvas, tank: &Tank, time: f32) {
    let size = tank.sprite_size();
    let origin = draw_pivot(size);
    let (dx, dy) = (tuning().shadow_dir_x * tuning().tank_shadow_offset, tuning().shadow_dir_y * tuning().tank_shadow_offset);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().tank_shadow_opacity * tank.alpha()) as u8);
    for (sheet, src, at, rot) in layers(tank, time, false) {
        c.blit(sheet, src, Rectangle::new(at.x + dx, at.y + dy, size, size), origin, rot, shadow);
    }
}

// The FIFO weapon-inventory rule (`weapon_queue`/`active_weapon`/
// `enqueue_weapon`) is pure `Tank` logic - no physics, no rendering - so it
// gets a real unit test rather than staying play-tested-only, same
// reasoning as `pathfind.rs`'s tests (see CLAUDE.md's testing section).
// Ammo is set directly here instead of going through `Game`'s pickup
// collection; the contract under test is the queue's, and the collection
// block's only obligations (enqueue before granting ammo, one call per
// pickup) are documented on `enqueue_weapon` itself.
#[cfg(test)]
mod weapon_queue_tests {
    use super::*;

    #[test]
    fn fifo_weapon_rotation() {
        let mut tank = Tank::default();
        assert_eq!(tank.active_weapon(), ActiveWeapon::Shell);

        // A first pickup arms immediately (nothing ahead of it in line).
        tank.enqueue_weapon(ActiveWeapon::Minigun);
        tank.minigun_ammo += 40;
        assert_eq!(tank.active_weapon(), ActiveWeapon::Minigun);

        // A later pickup queues *behind* the live weapon - it must not
        // hijack the trigger.
        tank.enqueue_weapon(ActiveWeapon::Laser);
        tank.laser_charges += 3;
        assert_eq!(tank.active_weapon(), ActiveWeapon::Minigun);

        // Depleting the live weapon hands the trigger to the next in line.
        tank.minigun_ammo = 0;
        assert_eq!(tank.active_weapon(), ActiveWeapon::Laser);

        // Re-collecting a spent weapon re-enters at the *back* of the
        // line, never resurrecting in its old front slot.
        tank.enqueue_weapon(ActiveWeapon::Minigun);
        tank.minigun_ammo += 40;
        assert_eq!(tank.active_weapon(), ActiveWeapon::Laser);

        // The rotation keeps advancing in pickup order, and only a fully
        // spent queue falls back to shells.
        tank.laser_charges = 0;
        assert_eq!(tank.active_weapon(), ActiveWeapon::Minigun);
        tank.minigun_ammo = 0;
        assert_eq!(tank.active_weapon(), ActiveWeapon::Shell);
    }

    #[test]
    fn topping_up_keeps_place_in_line() {
        let mut tank = Tank::default();
        tank.enqueue_weapon(ActiveWeapon::Plasma);
        tank.plasma_ammo += 4;
        tank.enqueue_weapon(ActiveWeapon::Minigun);
        tank.minigun_ammo += 40;
        // Re-collecting the still-stocked live weapon is a plain top-up: no
        // duplicate entry, no change to the order.
        tank.enqueue_weapon(ActiveWeapon::Plasma);
        tank.plasma_ammo += 4;
        assert_eq!(
            tank.weapon_queue,
            vec![ActiveWeapon::Plasma, ActiveWeapon::Minigun]
        );
        assert_eq!(tank.active_weapon(), ActiveWeapon::Plasma);
    }
}

#[cfg(test)]
mod shield_tests {
    use super::*;

    #[test]
    fn a_shielded_tank_takes_no_damage() {
        let mut tank = Tank { damage: 10.0, shield_hp: 1.0, ..Tank::default() };
        tank.take_damage(30.0, MAX_DAMAGE);
        assert_eq!(tank.damage, 10.0, "shield absorbs the whole hit");
        assert!(!tank.is_wreck());
    }

    #[test]
    fn damage_lands_and_caps_once_the_shield_is_gone() {
        let mut tank = Tank { damage: 10.0, shield_hp: 0.0, ..Tank::default() };
        tank.take_damage(30.0, MAX_DAMAGE);
        assert_eq!(tank.damage, 40.0);
        tank.take_damage(1000.0, MAX_DAMAGE - 1.0);
        assert_eq!(tank.damage, MAX_DAMAGE - 1.0, "capped at the caller's ceiling");
        assert!(!tank.is_wreck());
        tank.take_damage(1000.0, MAX_DAMAGE);
        assert!(tank.is_wreck());
    }
}

#[cfg(test)]
mod shield_ring_tests {
    use super::*;

    #[test]
    fn shield_charge_is_the_fraction_of_the_pool_left() {
        let capacity = tuning().shield_capacity;
        let with = |shield_hp: f32| Tank { shield_hp, ..Tank::default() }.shield_charge();
        assert_eq!(with(capacity), 1.0);
        assert!((with(capacity / 4.0) - 0.25).abs() < 1e-5);
        assert_eq!(with(capacity * 3.0), 1.0, "a pool pushed past the knob still reads as full");
        assert_eq!(with(0.0), 0.0);
    }

    /// Degrees a band's pieces cover once empty pieces are dropped.
    fn covered(pieces: [(f32, f32); 2]) -> f32 {
        pieces.iter().map(|&(a, b)| (b - a).max(0.0)).sum()
    }

    #[test]
    fn a_band_inside_a_full_window_is_drawn_whole() {
        let pieces = arc_pieces(30.0, 60.0, 360.0);
        assert_eq!(pieces[0], (30.0, 90.0));
        assert!(pieces[1].1 <= pieces[1].0, "nothing wrapped");
        assert_eq!(covered(pieces), 60.0);
    }

    #[test]
    fn a_band_straddling_twelve_o_clock_splits_and_still_covers_its_length() {
        let pieces = arc_pieces(330.0, 60.0, 360.0);
        assert_eq!(pieces[0], (330.0, 360.0));
        assert_eq!(pieces[1], (0.0, 30.0));
        assert_eq!(covered(pieces), 60.0);
    }

    #[test]
    fn bands_are_clipped_to_the_charge_window() {
        // Window of 45 degrees: a band starting at 30 keeps 15 of them ...
        assert_eq!(covered(arc_pieces(30.0, 60.0, 45.0)), 15.0);
        // ... a band past the window shows nothing ...
        assert_eq!(covered(arc_pieces(90.0, 60.0, 45.0)), 0.0);
        // ... and a wrapped band keeps only its wrapped start.
        assert_eq!(arc_pieces(330.0, 60.0, 20.0)[1], (0.0, 20.0));
        assert_eq!(covered(arc_pieces(330.0, 60.0, 20.0)), 20.0);
        // An empty window draws nothing at all.
        assert_eq!(covered(arc_pieces(0.0, 60.0, 0.0)), 0.0);
    }

    #[test]
    fn six_bands_cover_exactly_the_window() {
        for sweep in [0.0, 45.0, 180.0, 270.0, 360.0] {
            for base_hue in [0.0, 17.0, 200.0, 359.0] {
                let total: f32 = (0..6)
                    .map(|i| {
                        let from = (i as f32 * 60.0 - base_hue - HEALTH_RING_START_DEG).rem_euclid(360.0);
                        covered(arc_pieces(from, 60.0, sweep))
                    })
                    .sum();
                assert!((total - sweep).abs() < 1e-3, "sweep {sweep} hue {base_hue}: covered {total}");
            }
        }
    }
}

#[cfg(test)]
mod health_ring_tests {
    use super::*;

    fn enemy(damage: f32, hit_flash_timer: f32) -> Tank {
        Tank { owner: Owner::Enemy(1), damage, hit_flash_timer, ..Tank::default() }
    }

    #[test]
    fn ramp_steps_by_quarter() {
        assert_eq!(health_ring_step(1.0), 0);
        assert_eq!(health_ring_step(0.76), 0);
        assert_eq!(health_ring_step(0.75), 1);
        assert_eq!(health_ring_step(0.51), 1);
        assert_eq!(health_ring_step(0.5), 2);
        assert_eq!(health_ring_step(0.26), 2);
        assert_eq!(health_ring_step(0.25), 3);
        assert_eq!(health_ring_step(0.0), 3);
    }

    #[test]
    fn sweep_is_proportional_and_clamped() {
        assert_eq!(health_ring_sweep(1.0), 360.0);
        assert!((health_ring_sweep(0.4) - 144.0).abs() < 1e-3);
        assert_eq!(health_ring_sweep(0.0), 0.0);
        assert_eq!(health_ring_sweep(1.5), 360.0);
        assert_eq!(health_ring_sweep(-1.0), 0.0);
    }

    #[test]
    fn segments_keep_the_full_ring_density() {
        assert_eq!(health_ring_segments(360.0), 48);
        for sweep in [1.0, 7.5, 90.0, 144.0] {
            let segments = health_ring_segments(sweep);
            assert!(segments >= 1);
            assert!(segments >= (sweep / 90.0).ceil() as i32, "raylib's per-quadrant floor at {sweep} deg");
        }
    }

    #[test]
    fn health_fraction_counts_down_from_pristine() {
        assert_eq!(Tank::default().health_fraction(), 1.0);
        assert!((Tank { damage: 60.0, ..Tank::default() }.health_fraction() - 0.4).abs() < 1e-5);
        assert_eq!(Tank { damage: MAX_DAMAGE + 50.0, ..Tank::default() }.health_fraction(), 0.0);
    }

    #[test]
    fn enemy_ring_shows_for_the_hit_window_then_fades() {
        let (window, fade) = {
            let t = tuning();
            (t.health_ring_hit_seconds, t.health_ring_hit_fade_seconds)
        };
        assert_eq!(enemy_health_ring_visibility(&enemy(10.0, window)), 1.0);
        assert!((enemy_health_ring_visibility(&enemy(10.0, fade / 2.0)) - 0.5).abs() < 1e-5);
        assert_eq!(enemy_health_ring_visibility(&enemy(10.0, 0.0)), 0.0);
    }

    #[test]
    fn enemy_ring_stays_on_once_low_whatever_the_timer() {
        let low_damage = (1.0 - tuning().enemy_health_ring_below) * MAX_DAMAGE;
        assert_eq!(enemy_health_ring_visibility(&enemy(low_damage, 0.0)), 1.0);
        assert_eq!(enemy_health_ring_visibility(&enemy(low_damage - 1.0, 0.0)), 0.0);
    }

    #[test]
    fn no_ring_on_a_wreck() {
        assert_eq!(enemy_health_ring_visibility(&enemy(MAX_DAMAGE, 3.0)), 0.0);
        assert_eq!(player_health_ring_visibility(&Tank { damage: MAX_DAMAGE, ..Tank::default() }), 0.0);
    }

    #[test]
    fn rings_yield_to_the_shield() {
        // The fade is a fraction of charge now, not a countdown: a healthy
        // shield hides the health ring entirely, and one down to half its
        // fade threshold shows the hull half way back through.
        let capacity = tuning().shield_capacity;
        let fade = tuning().shield_glow_fade_fraction;
        let full = capacity * fade * 2.0;
        assert_eq!(enemy_health_ring_visibility(&Tank { shield_hp: full, ..enemy(60.0, 3.0) }), 0.0);
        assert_eq!(player_health_ring_visibility(&Tank { shield_hp: full, ..Tank::default() }), 0.0);
        let dropping = Tank { shield_hp: capacity * fade / 2.0, ..Tank::default() };
        assert!((player_health_ring_visibility(&dropping) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn player_ring_is_always_on_until_the_wreck() {
        assert_eq!(player_health_ring_visibility(&Tank::default()), 1.0);
        assert_eq!(player_health_ring_visibility(&Tank { damage: 99.0, ..Tank::default() }), 1.0);
    }
}

#[cfg(test)]
mod ring_tests {
    use super::*;

    #[test]
    fn ring_snaps_when_the_hull_is_far_away() {
        let mut tank = Tank { position: Position::new(500.0, 300.0), ..Tank::default() };
        tank.ease_ring_position(1.0 / 60.0);
        assert_eq!(tank.ring_position, tank.position);
    }

    #[test]
    fn ring_snaps_when_the_hull_is_far_away_and_drops_its_speed() {
        let mut tank = Tank { position: Position::new(500.0, 300.0), ..Tank::default() };
        tank.ring_velocity = Vec2::new(40.0, 0.0);
        tank.ease_ring_position(1.0 / 60.0);
        assert_eq!(tank.ring_velocity, Vec2::new(0.0, 0.0));
    }

    #[test]
    fn ring_hangs_back_first_then_catches_up_and_settles() {
        let dt = 1.0 / 60.0;
        // Start inside the leash so only the spring is being tested.
        let mut tank = Tank { position: Position::new(10.0, 0.0), ..Tank::default() };
        // Sleepy: after one frame it has barely moved, well behind a plain
        // exponential follow would be.
        tank.ease_ring_position(dt);
        assert!(tank.ring_position.x < 1.0, "ring should start lazily, got {}", tank.ring_position.x);
        // ...but it does get going.
        for _ in 0..10 {
            tank.ease_ring_position(dt);
        }
        assert!(tank.ring_position.x > 5.0, "ring should be on its way, got {}", tank.ring_position.x);
        // ...and settles on the hull within a couple of seconds.
        for _ in 0..120 {
            tank.ease_ring_position(dt);
        }
        assert!((tank.ring_position.x - 10.0).abs() < 0.5, "ring should have settled, at {}", tank.ring_position.x);
        assert!(tank.ring_velocity.x.abs() < 5.0, "ring should be at rest, v={}", tank.ring_velocity.x);
    }

    #[test]
    fn ring_trails_further_behind_a_faster_hull() {
        let dt = 1.0 / 60.0;
        let trail_at = |speed: f32| {
            let mut tank = Tank::default();
            for _ in 0..300 {
                tank.position.x += speed * dt;
                tank.ease_ring_position(dt);
            }
            tank.position.x - tank.ring_position.x
        };
        let slow = trail_at(40.0);
        let fast = trail_at(80.0);
        assert!(slow > 0.0, "ring should trail a moving hull, got {slow}");
        assert!(fast > slow * 1.5, "faster hull should stretch the trail: slow={slow} fast={fast}");
    }

    #[test]
    fn ring_trail_is_leashed_at_any_speed() {
        let dt = 1.0 / 60.0;
        let mut tank = Tank::default();
        let leash = tuning().tank_ring_max_trail_px;
        for _ in 0..300 {
            tank.position.x += 400.0 * dt;
            tank.ease_ring_position(dt);
            let trail = tank.position.x - tank.ring_position.x;
            assert!(trail <= leash + 1e-3, "trail {trail} exceeds leash {leash}");
        }
    }
}

#[cfg(test)]
mod chassis_tests {
    use super::*;
    use crate::tuning::TANK_NAMES;

    /// `TankKind`'s declaration order *is* the sprite sheet's row order, and
    /// its names are `tuning::TANK_NAMES` - `row()`/`name()` and every
    /// `[_; 12]` tuning row indexed by `Tank::row` silently disagree the
    /// moment one list is reordered or renamed without the other.
    #[test]
    fn chassis_name_order() {
        assert_eq!(TankKind::ALL.len(), TANK_NAMES.len());
        for (i, kind) in TankKind::ALL.iter().enumerate() {
            assert_eq!(kind.row(), i as i32);
            assert_eq!(kind.name(), TANK_NAMES[i]);
            assert_eq!(TankKind::from_row(i as i32), Some(*kind));
        }
        assert_eq!(TankKind::from_row(-1), None);
        assert_eq!(TankKind::from_row(TANK_NAMES.len() as i32), None);
    }

    /// The sheet row is the chassis inside the owner's team block: enemies
    /// draw the first block, the players the recoloured copies after it,
    /// and the chassis row itself never moves (every per-chassis table is
    /// indexed by it).
    #[test]
    fn sheet_row_is_the_chassis_in_the_owners_block() {
        let tank = |owner, row| Tank { owner, row, ..Tank::default() };
        assert_eq!(tank(Owner::Enemy(3), 5).sheet_row(), 5);
        assert_eq!(tank(Owner::Player(0), 5).sheet_row(), 5 + TANK_ROWS_PER_TEAM);
        assert_eq!(tank(Owner::Player(1), 5).sheet_row(), 5 + 2 * TANK_ROWS_PER_TEAM);
        assert_eq!(tank(Owner::Player(2), 5).sheet_row(), 5 + 3 * TANK_ROWS_PER_TEAM);
        assert_eq!(tank(Owner::Player(3), 5).sheet_row(), 5 + 4 * TANK_ROWS_PER_TEAM);
        // A seat past the fourth borrows player 1's block.
        assert_eq!(tank(Owner::Player(4), 5).sheet_row(), 5 + TANK_ROWS_PER_TEAM);
        assert_eq!(tank(Owner::Player(1), 11).row, 11);
        // The builder's markers: the middle frame of the scout's hull and
        // turret cells, in the seat's block.
        let inset = (TANK_SPRITE_SIZE - TANK_FRAME_SIZE) / 2.0;
        let [hull, turret] = icon_source_recs(1);
        assert_eq!(hull.y, (2 * TANK_ROWS_PER_TEAM) as f32 * TANK_SPRITE_SIZE + inset);
        assert_eq!((hull.x, hull.width, hull.height), (inset, TANK_FRAME_SIZE, TANK_FRAME_SIZE));
        assert_eq!((turret.x, turret.y), (TANK_TURRET_COL as f32 * TANK_SPRITE_SIZE + inset, hull.y));
        // The MAP panel's chassis: the same window of the chassis's own
        // cells, raised to hold the longest barrels.
        let [hull, turret] = chassis_icon_source_recs(TankKind::Titan, 1);
        let row = TankKind::Titan.row() + 2 * TANK_ROWS_PER_TEAM;
        assert_eq!((hull.y, turret.y), (row as f32 * TANK_SPRITE_SIZE + inset - CHASSIS_ICON_RAISE, hull.y));
        assert_eq!((hull.x, turret.x), (inset, TANK_TURRET_COL as f32 * TANK_SPRITE_SIZE + inset));
        assert_eq!((hull.width, turret.height), (TANK_FRAME_SIZE, TANK_FRAME_SIZE));
    }

    /// The hull steps through the damage tiers at the thresholds and keeps
    /// its track frame at every live tier; a wreck is its rolled cell.
    #[test]
    fn hull_and_turret_cells_follow_damage_frame_and_recoil() {
        let mut tank = Tank { hull_frame: 3, ..Tank::default() };
        assert_eq!(tank.hull_col(), 3);
        tank.damage = TANK_DAMAGE_TIERS[0];
        assert_eq!(tank.hull_col(), TANK_TRACK_FRAMES + 3);
        tank.damage = TANK_DAMAGE_TIERS[2];
        assert_eq!(tank.hull_col(), 3 * TANK_TRACK_FRAMES + 3);
        assert_eq!(tank.turret_col(), TANK_TURRET_COL + 3 * TANK_TURRET_POSES);
        tank.recoil_pose = 2;
        assert_eq!(tank.turret_col(), TANK_TURRET_COL + 3 * TANK_TURRET_POSES + 2);
        tank.damage = MAX_DAMAGE;
        tank.wreck_col = Some(TANK_WRECK_COLS[2]);
        assert_eq!(tank.hull_col(), TANK_WRECK_COLS[2]);
        assert_eq!(tank.turret_col(), TANK_BROKEN_TURRET_COL);
        assert!(!tank.turret_thrown());
        tank.wreck_col = Some(TANK_WRECK_COLS[0]);
        assert!(tank.turret_thrown());
    }

    /// A single barrel kicks and returns; a twin kicks its first barrel,
    /// then its second as the second shell leaves, then rests.
    #[test]
    fn recoil_walks_its_poses_and_rests() {
        let t = crate::tuning::tuning();
        let step = t.tank_recoil_seconds * 0.6;
        let mut single = Tank { row: TankKind::Scout.row(), ..Tank::default() };
        single.kick(false);
        assert_eq!(single.recoil_pose, 1);
        let mut poses = vec![single.recoil_pose];
        for _ in 0..8 {
            single.tick_recoil(step);
            poses.push(single.recoil_pose);
        }
        assert!(poses.contains(&2), "a single barrel returns through pose 2: {poses:?}");
        assert_eq!(*poses.last().unwrap(), 0, "and rests: {poses:?}");
        let mut twin = Tank { row: TankKind::Assault.row(), ..Tank::default() };
        twin.kick(false);
        let mut poses = vec![twin.recoil_pose];
        for _ in 0..8 {
            twin.tick_recoil(step);
            poses.push(twin.recoil_pose);
        }
        assert!(poses.windows(2).any(|w| w == [1, 2]), "the second barrel follows the first: {poses:?}");
        assert_eq!(*poses.last().unwrap(), 0, "and rests: {poses:?}");
    }

    /// The art's main-gun muzzles (tank_art, generated from the design)
    /// sit within a design pixel of where the game spawns shells, and at
    /// the twin offsets it fires them from.
    #[test]
    fn art_muzzles_match_the_spawn_points() {
        let t = crate::tuning::tuning();
        for kind in TankKind::ALL {
            let row = kind.row() as usize;
            let muzzles = crate::tank_art::GUN_MUZZLES[row];
            let lat = t.tank_barrel_lateral_offset[row];
            assert_eq!(muzzles.len(), if lat > 0.0 { 2 } else { 1 }, "{kind:?}: barrel count");
            for &(x, y) in muzzles {
                assert!((-y - t.tank_muzzle_forward_offset[row]).abs() <= 1.0, "{kind:?}: muzzle {y} vs {}", t.tank_muzzle_forward_offset[row]);
                assert!((x.abs() - lat).abs() < 0.01, "{kind:?}: lateral {x} vs {lat}");
            }
        }
    }

    /// A map's `tank = "titan"` key and `--tank titan` spell a chassis the
    /// same way - serde's lowercase rename and `ValueEnum`'s.
    #[test]
    fn serde_spelling_is_the_cli_spelling() {
        for kind in TankKind::ALL {
            let json = serde_json::to_string(&kind).expect("serializes");
            assert_eq!(json, format!("\"{}\"", kind.name()));
            let back: TankKind = serde_json::from_str(&json).expect("round-trips");
            assert_eq!(back, kind);
        }
    }
}

#[cfg(test)]
mod credit_tests {
    use super::*;

    /// A seat's hit on an enemy is that seat's to be credited with; an
    /// enemy's hit credits nobody, and neither does any hit on a seat.
    #[test]
    fn only_a_seats_hit_on_an_enemy_is_credited() {
        let mut enemy = Tank { owner: Owner::Enemy(3), ..Tank::default() };
        enemy.credit(Owner::Enemy(4));
        assert_eq!(enemy.last_hit_by, None);
        enemy.credit(Owner::Player(1));
        assert_eq!(enemy.last_hit_by, Some(1));
        enemy.credit(Owner::Player(0));
        assert_eq!(enemy.last_hit_by, Some(0), "the last seat to hit it");
        enemy.credit(Owner::Enemy(4));
        assert_eq!(enemy.last_hit_by, Some(0), "an enemy's hit leaves the credit where it was");
        let mut seat = Tank { owner: Owner::Player(1), ..Tank::default() };
        seat.credit(Owner::Player(0));
        assert_eq!(seat.last_hit_by, None);
    }
}
