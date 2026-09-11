use crate::tuning::tuning;
use clap::ValueEnum;
use rapier2d::prelude::RigidBodyHandle;
use serde::{Deserialize, Serialize};
use sola_raylib::prelude::*;

use crate::laser::LaserVariant;
use crate::plasma::PlasmaVariant;
use crate::shell::Owner;
use crate::{
    MAX_DAMAGE,
    MINIGUN_MOUNT_SCALE,
    MINIGUN_MOUNT_TEXTURE_SIZE,
    Position,
    TANK_BROKEN_TURRET_COL,
    TANK_HULL_BBOX_BY_ROW,
    TANK_HULL_DISABLED_COL,
    TANK_HULL_DISABLED_DAMAGE,
    TANK_HULL_FRACTION,
    TANK_HULL_LIGHT_COL,
    TANK_HULL_LIGHT_DAMAGE,
    TANK_HULL_TRACK_COLS,
    TANK_MOVE_BBOX_FRACTION,
    TANK_PIVOT_REAR_FRACTION,
    TANK_ROWS_PER_TEAM,
    TANK_TEXTURE_SIZE,
    TANK_TURRET_BBOX_BY_ROW,
    TANK_TURRET_COL,
    TANK_WRECK_COLS,
};

/// The four movement/facing directions. rotation 0 == up, clockwise positive,
/// matching the sprite orientation and shell-spawn math.
#[derive(Clone, Copy, PartialEq, Debug)]
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

    /// Unit movement vector (screen space: +x right, +y down).
    pub fn vec(self) -> Vector2 {
        match self {
            Dir::Up => Vector2::new(0.0, -1.0),
            Dir::Down => Vector2::new(0.0, 1.0),
            Dir::Left => Vector2::new(-1.0, 0.0),
            Dir::Right => Vector2::new(1.0, 0.0),
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

/// Which weapon a tank's next trigger-pull fires - see `Tank::active_weapon`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveWeapon {
    Laser,
    Plasma,
    Minigun,
    Shell,
}

impl ActiveWeapon {
    /// Lower-case name for tooling/JSON (`Event::Fired`, the dev server).
    pub fn name(self) -> &'static str {
        match self {
            ActiveWeapon::Laser => "laser",
            ActiveWeapon::Plasma => "plasma",
            ActiveWeapon::Minigun => "minigun",
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
    /// Row in damage.png this tank's damage overlay is drawn from
    /// (0..DAMAGE_VARIANTS). Rolled once at spawn (see Game::init) and fixed
    /// for the tank's whole life, so its damage sequence reads as one
    /// consistent flavour rather than switching palettes between stages.
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
    pub ring_velocity: Vector2,
    /// Seconds accumulated toward the minigun barrel-cluster overlay's next
    /// "hot barrel" frame swap (see `draw_minigun_mount`), advanced while
    /// `minigun_burst` is active (see `tick_minigun_spin`) and held in place
    /// - not reset to 0 - the rest of the time, so the mount doesn't
    /// visually snap back to frame 0 between bursts. Wrapped to
    /// `MINIGUN_CYCLE_SECONDS * 3.0` (one full lap of the 3 frames) rather
    /// than growing unbounded.
    ///
    /// This deliberately drives a discrete frame swap, not a continuous
    /// rotation: `minigun_mount.png`'s barrels point along the ground plane
    /// toward the target, so their real rotation axis is edge-on to this
    /// game's top-down camera, not face-on to it - spinning the whole
    /// sprite in the screen plane would read as a helicopter rotor seen
    /// from above, not a side-mounted minigun. Cycling which barrel reads
    /// as freshly-fired fakes the same "rounds cycling through" idea
    /// correctly for this camera angle instead. See
    /// `tools/spritegen/gen_minigun_mount.py`'s module doc comment.
    pub minigun_cycle_timer: f32,
    /// Index into TANK_HULL_TRACK_COLS (0..4) picking which tread-animation
    /// hull frame is currently drawn - see `hull_col`/
    /// TANK_HULL_TRACK_FRAME_DISTANCE. Only consulted while the tank is alive
    /// and below TANK_HULL_LIGHT_DAMAGE; damaged/wrecked hulls hold a fixed
    /// frame instead.
    pub hull_frame: i32,
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
    /// How much to scale the 32x32 sprite when drawn.
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
    /// Seconds remaining on a `pickup::PickupKind::Shield` - while positive
    /// `take_damage` is a no-op and `draw_tank_shield` draws the rainbow
    /// ring. Set (not added to) on pickup, same refresh-not-stack rule as
    /// `speed_boost_timer`.
    pub shield_timer: f32,
    /// Seconds accumulated toward recharging the next shell.
    pub recharge_timer: f32,
    /// Seconds remaining before this tank may fire again (player only - see
    /// PLAYER_FIRE_INTERVAL; enemies are gated separately by Ai's own
    /// fire_timer). Ticked down alongside ram_cooldown every frame regardless
    /// of owner, since it's harmless/unused idle state for enemies.
    pub fire_cooldown: f32,
    /// Seconds remaining before this tank can take ramming damage again.
    pub ram_cooldown: f32,
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
    pub velocity: Vector2,
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
            damage_variant: 0,
            position: Position::default(),
            rotation: 0.0,
            visual_rotation: 0.0,
            turret_visual_rotation: 0.0,
            ring_position: Position::default(),
            ring_velocity: Vector2::new(0.0, 0.0),
            minigun_cycle_timer: 0.0,
            hull_frame: 0,
            hull_anim_accum: 0.0,
            wreck_col: None,
            scale: 2.0, // 3.0,
            speed_scale: 1.0,
            damage: 0.0,
            shells_ammo: tuning().max_shells,
            laser_charges: 0,
            laser_variant: LaserVariant::Red,
            minigun_ammo: 0,
            plasma_ammo: 0,
            plasma_variant: PlasmaVariant::Teal,
            weapon_queue: Vec::new(),
            speed_boost_timer: 0.0,
            shield_timer: 0.0,
            recharge_timer: 0.0,
            fire_cooldown: 0.0,
            ram_cooldown: 0.0,
            hit_flash_timer: 0.0,
            wreck_timer: 0.0,
            despawn_timer: None,
            track_accum: 0.0,
            track_mark_count: 0,
            track_wobble_amp: 0.0,
            track_wobble_freq: 0.0,
            track_wobble_phase: 0.0,
            track_scale_jitter: 1.0,
            velocity: Vector2::new(0.0, 0.0),
            body: None,

            owner: Owner::Player(0),
        }
    }
}

impl Tank {
    /// Side length of this tank on screen (square sprite).
    pub fn size(&self) -> f32 {
        TANK_TEXTURE_SIZE * self.scale
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

    /// True while a rainbow shield is active (see `shield_timer`).
    pub fn is_shielded(&self) -> bool {
        self.shield_timer > 0.0
    }

    /// How much of a shield's run is left, 0..=1: `shield_timer` over
    /// `shield_duration_seconds` (what a pickup or a spawn roll sets it to),
    /// clamped so a timer pushed past the knob still reads as full.
    pub fn shield_charge(&self) -> f32 {
        let duration = tuning().shield_duration_seconds;
        if duration > 0.0 { (self.shield_timer / duration).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// The one way damage lands on a tank: adds `amount`, capped at `cap`
    /// (MAX_DAMAGE, or one below it for the player's frog bites). A no-op
    /// while shielded - the shield absorbs every source, shells, bullets,
    /// plasma, laser, ram, explosions and the frog alike. Callers keep their
    /// hit flash/knockback/alert side effects, so a blocked shot still
    /// visibly lands; only the health change is swallowed.
    pub fn take_damage(&mut self, amount: f32, cap: f32) {
        if self.is_shielded() {
            return;
        }
        self.damage = (self.damage + amount).min(cap);
    }

    /// Who this tank is as a projectile owner.
    pub fn owner(&self) -> Owner {
        self.owner
    }

    /// This tank's owner slot, the number events, snapshots and the dev
    /// tools address it by: players first (`0` for player 1, `1` for
    /// player 2 in a two-player round), then the enemies, counting up from
    /// the first free number - `1` in a single-player round, `2` with two
    /// players (`Game::first_enemy_slot`). Unique for the round.
    pub fn owner_slot(&self) -> usize {
        self.owner.slot()
    }

    /// A human player, whichever one.
    pub fn is_player(&self) -> bool {
        self.owner.is_player()
    }

    /// Which player this is, 0 or 1, if a player at all.
    pub fn player_index(&self) -> Option<u8> {
        match self.owner {
            Owner::Player(i) => Some(i),
            Owner::Enemy(_) => None,
        }
    }

    /// The row of the sprite sheet this tank draws from: its chassis row
    /// inside the block for its team - the enemy block first, then player
    /// 1's and player 2's recoloured copies (`TANK_ROWS_PER_TEAM`).
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

    /// Which atlas column to draw this tank's hull from - a four-tier
    /// escalation matching the sheet's own damage ladder: the rolled wreck
    /// variant once it's a wreck (`wreck_col`, falling back to the first
    /// wreck variant on the off chance this is read before that roll
    /// happens), the disabled art once it's taken heavy but non-fatal damage
    /// (TANK_HULL_DISABLED_DAMAGE), the cosmetic "light" art once it's taken
    /// moderate damage (TANK_HULL_LIGHT_DAMAGE) - still fully mobile, so this
    /// tier is static art rather than consulting `hull_frame` - otherwise
    /// whichever tread-animation frame `hull_frame` currently points at.
    pub fn hull_col(&self) -> i32 {
        if self.is_wreck() {
            self.wreck_col.unwrap_or(TANK_WRECK_COLS[0])
        } else if self.damage >= TANK_HULL_DISABLED_DAMAGE {
            TANK_HULL_DISABLED_COL
        } else if self.damage >= TANK_HULL_LIGHT_DAMAGE {
            TANK_HULL_LIGHT_COL
        } else {
            TANK_HULL_TRACK_COLS[self.hull_frame as usize]
        }
    }

    /// Which atlas column to draw this tank's turret from: the severed/
    /// broken turret once it's a wreck, otherwise the intact turret.
    pub fn turret_col(&self) -> i32 {
        if self.is_wreck() {
            TANK_BROKEN_TURRET_COL
        } else {
            TANK_TURRET_COL
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
    fn weapon_ammo(&self, weapon: ActiveWeapon) -> i32 {
        match weapon {
            ActiveWeapon::Laser => self.laser_charges,
            ActiveWeapon::Plasma => self.plasma_ammo,
            ActiveWeapon::Minigun => self.minigun_ammo,
            ActiveWeapon::Shell => self.shells_ammo,
        }
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

    /// Which of `minigun_mount.png`'s 3 "hot barrel" frames to draw right
    /// now - see `minigun_cycle_timer`'s doc comment for why this is a
    /// discrete frame index, not a rotation angle.
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
            self.ring_velocity = Vector2::new(0.0, 0.0);
            return;
        }
        let t = tuning();
        let omega = t.tank_ring_spring_hz * std::f32::consts::TAU;
        if omega <= 0.0 {
            self.ring_position = self.position;
            self.ring_velocity = Vector2::new(0.0, 0.0);
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
    /// the second box the projectile hit test checks, and what the "I" key
    /// debug inspect overlay (`game.rs::draw_tank_inspect`) draws.
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
        let half = TANK_TEXTURE_SIZE * 0.5;
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
    /// clearance margins - see `battlefield::max_tank_avoidance_radius`.
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
            self.velocity = Vector2::new(step.x * speed, step.y * speed);
        } else {
            self.velocity = Vector2::new(0.0, 0.0);
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
fn draw_pivot(size: f32) -> Vector2 {
    Vector2::new(size / 2.0, size / 2.0 + size * TANK_PIVOT_REAR_FRACTION)
}

/// Which block of the sheet a tank draws from: 0 for an enemy, 1 and 2 for
/// the two players' recoloured copies.
fn sheet_block(player: Option<u8>) -> i32 {
    match player {
        None => 0,
        Some(i) => 1 + i as i32,
    }
}

/// Source rectangle for a fixed representative tank sprite - the Scout
/// chassis's idle hull frame (col 0) in `player`'s team colour - used by
/// the map editor's start-point palette icons and placed-cell markers,
/// which need one fixed tank sprite rather than any particular round's
/// rolled chassis, in the colour that tank will actually be.
pub fn icon_source_rec(player: u8) -> Rectangle {
    source_rec(sheet_block(Some(player)) * TANK_ROWS_PER_TEAM, 0)
}

/// Source rectangle for the tank at (row, col) inside the atlas.
fn source_rec(row: i32, col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * TANK_TEXTURE_SIZE,
        row as f32 * TANK_TEXTURE_SIZE,
        TANK_TEXTURE_SIZE,
        TANK_TEXTURE_SIZE,
    )
}

/// Draw a single tank sprite from the atlas at its center position, scaled
/// and rotated. Hull and turret are two separate layers in the atlas (see
/// `hull_col`/`turret_col` for which column each picks, depending on
/// animation/damage state) drawn hull-first-then-turret at the same
/// dest/origin but each at its own eased angle (`visual_rotation` for the
/// hull, `turret_visual_rotation` for the turret) - the turret still just
/// chases the tank's commanded `rotation`, not an independent aim target, but
/// it does so faster than the hull so it visibly leads a turn.
pub fn draw_tank(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    let hull_src = source_rec(tank.sheet_row(), tank.hull_col());
    let turret_src = source_rec(tank.sheet_row(), tank.turret_col());
    let size = tank.size();

    // dest is placed at the tank's position; origin is the rear-shifted
    // pivot (see `draw_pivot`), not the sprite's exact middle.
    let dest = Rectangle::new(tank.position.x, tank.position.y, size, size);
    let origin = draw_pivot(size);

    let tint = tank.tint();
    d.draw_texture_pro(texture, hull_src, dest, origin, tank.visual_rotation, tint);
    d.draw_texture_pro(
        texture,
        turret_src,
        dest,
        origin,
        tank.turret_visual_rotation,
        tint,
    );
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
const BLACK: Color = Color::new(0x25, 0x25, 0x25, 255);
/// The two players' identity colours (docs/player-indicator-improvements.md):
/// player 1 sky blue, player 2 hot pink - the base step of the team ramp
/// the sheet's player blocks are painted in, deliberately off the Puny
/// Palette because every one of its hue families is already an enemy
/// hull. The hull, the ground ring, the HUD readouts and button, the
/// editor's start markers and the round-start locate cue all draw from
/// this one pair so the three surfaces read as one identity.
pub const TEAM_COLORS: [Color; 2] = [Color::new(0x4D, 0x9B, 0xE6, 255), Color::new(0xF0, 0x4F, 0x78, 255)];
/// The light step of each team ramp: the sheet's accent, and the ring's
/// full-health colour so a healthy ring reads brighter than the hull.
const TEAM_LIGHT: [Color; 2] = [Color::new(0x8F, 0xD3, 0xFF, 255), Color::new(0xED, 0x80, 0x99, 255)];

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
    /// Player 1: the team's light and base blues while healthy, then the
    /// same gold and bright red as `White` for the shared danger steps.
    Blue,
    /// Player 2: the same shape in the team's pinks.
    Pink,
}

impl HealthRamp {
    const WHITE_STEPS: [Color; 4] = [Color::WHITE, GOLD_BRIGHT, RED_BRIGHT, RED_DEEP];
    const RED_STEPS: [Color; 4] = [RED_BRIGHT, RED_DEEP, RED_DK, RED_DARKEST];
    const BLUE_STEPS: [Color; 4] = [TEAM_LIGHT[0], TEAM_COLORS[0], GOLD_BRIGHT, RED_BRIGHT];
    const PINK_STEPS: [Color; 4] = [TEAM_LIGHT[1], TEAM_COLORS[1], GOLD_BRIGHT, RED_BRIGHT];

    /// The ramp for player `index` (0 or 1).
    pub fn player(index: u8) -> Self {
        if index == 0 { Self::Blue } else { Self::Pink }
    }

    /// The step colour for `frac` remaining health.
    pub fn color(self, frac: f32) -> Color {
        let steps = match self {
            Self::White => Self::WHITE_STEPS,
            Self::Red => Self::RED_STEPS,
            Self::Blue => Self::BLUE_STEPS,
            Self::Pink => Self::PINK_STEPS,
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
            Self::Blue => TEAM_COLORS[0],
            Self::Pink => TEAM_COLORS[1],
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
pub fn draw_ground_ring(d: &mut impl RaylibDraw, tank: &Tank, time: f32, style: RingStyle, fade: f32) {
    draw_ground_ring_scaled(d, tank.ring_position, tank.size(), tank.anim_phase(), time, style, fade, ring_scale(tank));
}

/// How much larger than the shared ring a tank's rings are drawn: the
/// players' `player_ring_radius_scale`, 1 for an enemy.
fn ring_scale(tank: &Tank) -> f32 {
    if tank.is_player() { tuning().player_ring_radius_scale } else { 1.0 }
}

/// `draw_ground_ring` for anything that is not a tank - a frog's side
/// marker (`frog::draw_frog_ring`): the same ring at `center`, sized from
/// `size` (the sprite's on-screen side length) exactly as a tank's is from
/// `Tank::size`, with `phase` offsetting the rainbow's breathing.
pub fn draw_ground_ring_at(
    d: &mut impl RaylibDraw,
    center: Position,
    size: f32,
    phase: f32,
    time: f32,
    style: RingStyle,
    fade: f32,
) {
    draw_ground_ring_scaled(d, center, size, phase, time, style, fade, 1.0);
}

/// `draw_ground_ring_at` with the radius scaled by `radius_scale` at the
/// *unscaled* band thickness, so a larger ring is a wider halo, not a
/// fatter one (the players' rings, `player_ring_radius_scale`).
#[allow(clippy::too_many_arguments)]
pub fn draw_ground_ring_scaled(
    d: &mut impl RaylibDraw,
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
            d.draw_circle_v(center, radius - thickness, with_alpha(fill, disc_alpha));
            let sweep = health_ring_sweep(charge);
            if sweep < 360.0 {
                let gap = with_alpha(base, base.a as f32);
                let segments = health_ring_segments(360.0 - sweep);
                let end = HEALTH_RING_START_DEG + sweep;
                d.draw_ring(center, radius - thickness, radius, end, HEALTH_RING_START_DEG + 360.0, segments, gap);
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
                        d.draw_ring(center, radius - thickness, radius, start, end, segments, color);
                    }
                }
            }
        }
        RingStyle::Solid(color) => {
            d.draw_circle_v(center, radius - thickness, with_alpha(color, disc_alpha));
            d.draw_ring(center, radius - thickness, radius, 0.0, 360.0, 48, with_alpha(color, band_alpha));
        }
        RingStyle::Gauge { frac, ramp, base } => {
            let fill = ramp.color(frac);
            d.draw_circle_v(center, radius - thickness, with_alpha(fill, disc_alpha));
            let sweep = health_ring_sweep(frac);
            let end = HEALTH_RING_START_DEG + sweep;
            // The missing part first and the health arc over it, so the
            // arc's end cap wins at the seam. raylib draws nothing for a
            // zero-width arc but mirrors a reversed one, hence the guards.
            if sweep < 360.0 {
                let gap = with_alpha(base, base.a as f32);
                let segments = health_ring_segments(360.0 - sweep);
                d.draw_ring(center, radius - thickness, radius, end, HEALTH_RING_START_DEG + 360.0, segments, gap);
            }
            if sweep > 0.0 {
                let arc = with_alpha(fill, band_alpha);
                let segments = health_ring_segments(sweep);
                d.draw_ring(center, radius - thickness, radius, HEALTH_RING_START_DEG, end, segments, arc);
            }
        }
    }
}

/// How far into its final fade-out a tank's shield is: 1 while it has more
/// than `shield_glow_fade_seconds` left, falling to 0 as it expires, 0 when
/// there is no shield at all. Drives the shield ring's opacity and, inverted,
/// the health ring's (`player_health_ring_visibility`,
/// `enemy_health_ring_visibility`), so the two cross-fade instead of stacking.
fn shield_visibility(tank: &Tank) -> f32 {
    if !tank.is_shielded() {
        return 0.0;
    }
    let fade_seconds = tuning().shield_glow_fade_seconds;
    if fade_seconds > 0.0 { (tank.shield_timer / fade_seconds).min(1.0) } else { 1.0 }
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

/// Draw the rainbow shield ring while `Tank::shield_timer` is running: a
/// `RingStyle::Rainbow` ground ring whose hue cycles at `shield_glow_hue_hz`
/// (offset by `anim_phase` so neighbouring tanks don't cycle in lockstep),
/// its bands covering the fraction of shield time left
/// (`Tank::shield_charge`) from 12 o'clock clockwise and the rest drawn as
/// the tank's health ring draws its missing part - dimmed white for the
/// player, a dark band for an enemy - so it visibly runs down; fading out
/// over the final `shield_glow_fade_seconds`.
pub fn draw_tank_shield(d: &mut impl RaylibDraw, tank: &Tank, time: f32) {
    if tank.is_wreck() {
        return;
    }
    let base_hue = (time * tuning().shield_glow_hue_hz * 360.0 + tank.anim_phase() * 360.0).rem_euclid(360.0);
    let base = match tank.owner() {
        Owner::Player(i) => with_opacity(TEAM_COLORS[i as usize & 1], tuning().player_ring_opacity * tuning().health_ring_base_opacity),
        Owner::Enemy(_) => with_opacity(BLACK, tuning().health_ring_gap_opacity),
    };
    let style = RingStyle::Rainbow { base_hue, charge: tank.shield_charge(), base };
    draw_ground_ring(d, tank, time, style, shield_visibility(tank));
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
/// as the shield (same thickness, `player_ring_radius_scale` wider, minus
/// the breathing), the remaining-health arc in the player's team ramp
/// (`HealthRamp::player`) at `player_ring_opacity` and the rest of the
/// circle in the team colour dimmed to `health_ring_base_opacity` of that,
/// so each human tank always has a steady full halo in its own colour that
/// also reads its health. Yields to the shield ring while one is up and
/// disappears with the wreck (`player_health_ring_visibility`). An enemy
/// handed to this draws as player 1.
pub fn draw_player_ring(d: &mut impl RaylibDraw, tank: &Tank, time: f32) {
    let ramp = HealthRamp::player(tank.player_index().unwrap_or(0));
    let base = with_opacity(ramp.base(), tuning().player_ring_opacity * tuning().health_ring_base_opacity);
    let style = RingStyle::Gauge { frac: tank.health_fraction(), ramp, base };
    draw_ground_ring(d, tank, time, style, player_health_ring_visibility(tank));
}

/// Draw an enemy's health ring: the same gauge in the enemy frog's all-red
/// ramp, its missing part a dark band at `health_ring_gap_opacity`, so a
/// just-hit enemy at full health reads hostile rather than borrowing a
/// player's colour. Shown per `enemy_health_ring_visibility` - after a
/// hit, or for good once low.
pub fn draw_enemy_ring(d: &mut impl RaylibDraw, tank: &Tank, time: f32) {
    let base = with_opacity(BLACK, tuning().health_ring_gap_opacity);
    let style = RingStyle::Gauge { frac: tank.health_fraction(), ramp: HealthRamp::Red, base };
    draw_ground_ring(d, tank, time, style, enemy_health_ring_visibility(tank));
}

/// The round-start locate cue's ripple: for `player_locate_seconds` after
/// the round became playable (`elapsed`, which is `Game::time` - it does
/// not run behind the mission banner) a team-coloured ring swells from the
/// player's own ring out to 1.6x its radius and fades as it goes,
/// `player_locate_pulse_hz` times a second. Drawn under the hull like the
/// other rings; `draw_player_label` is the cue's other half. Nothing for a
/// wreck, an enemy, or once the window has passed.
pub fn draw_player_locate(d: &mut impl RaylibDraw, tank: &Tank, time: f32, elapsed: f32) {
    let Some(index) = tank.player_index() else { return };
    if tank.is_wreck() || !player_locate_active(elapsed) {
        return;
    }
    let phase = (elapsed * tuning().player_locate_pulse_hz).fract();
    let color = with_opacity(TEAM_COLORS[index as usize & 1], (1.0 - phase) * tuning().player_ring_opacity);
    let scale = ring_scale(tank) * (1.0 + 0.6 * phase);
    draw_ground_ring_scaled(d, tank.ring_position, tank.size(), tank.anim_phase(), time, RingStyle::Solid(color), 1.0, scale);
}

/// Whether the locate cue is still showing `elapsed` seconds into play.
pub fn player_locate_active(elapsed: f32) -> bool {
    elapsed < tuning().player_locate_seconds
}

/// The locate cue's label, `P1`/`P2` in the team colour just above the
/// hull, drawn over everything so a crowd cannot cover it. Same window as
/// `draw_player_locate`.
pub fn draw_player_label(d: &mut impl RaylibDraw, tank: &Tank, elapsed: f32) {
    let Some(index) = tank.player_index() else { return };
    if tank.is_wreck() || !player_locate_active(elapsed) {
        return;
    }
    let text = if index == 0 { "P1" } else { "P2" };
    let size = crate::hud::HUD_TEXT_SIZE;
    // The HUD's fixed cell width for this size; measuring needs the handle,
    // which nothing in a draw pass has.
    let w = text.len() as i32 * crate::hud::CHAR_W;
    let x = (tank.position.x - w as f32 / 2.0).round() as i32;
    let y = (tank.position.y - tank.size() / 2.0 - size as f32 - 4.0).round() as i32;
    let color = TEAM_COLORS[index as usize & 1];
    d.draw_text(text, x + 1, y + 1, size, BLACK);
    d.draw_text(text, x, y, size, color);
}

/// Draw this tank's drop shadow: the same two layers (each at its own eased
/// angle, matching `draw_tank`), offset toward a fixed screen-space direction
/// and tinted flat black - see docs/sprite-shadows-design.md. Must be called
/// *before* `draw_tank` so the real sprite draws on top of its own shadow. No
/// wreck/dead special-casing needed - a burnt-out hulk is still a solid
/// object sitting on the ground.
pub fn draw_tank_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    let hull_src = source_rec(tank.sheet_row(), tank.hull_col());
    let turret_src = source_rec(tank.sheet_row(), tank.turret_col());
    let size = tank.size();

    let dest = Rectangle::new(
        tank.position.x + tuning().shadow_dir_x * tuning().tank_shadow_offset,
        tank.position.y + tuning().shadow_dir_y * tuning().tank_shadow_offset,
        size,
        size,
    );
    let origin = draw_pivot(size);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().tank_shadow_opacity * tank.alpha()) as u8);

    d.draw_texture_pro(texture, hull_src, dest, origin, tank.visual_rotation, shadow);
    d.draw_texture_pro(
        texture,
        turret_src,
        dest,
        origin,
        tank.turret_visual_rotation,
        shadow,
    );
}

/// Draw the minigun barrel-cluster overlay on top of this tank's turret, if
/// it currently has the weapon (`minigun_ammo > 0`) and isn't a wreck - the
/// existing broken-turret art (`turret_col`) already communicates
/// "destroyed" on its own, so this simply stops drawing once `is_wreck()`
/// rather than authoring a separate broken-minigun asset. Visible whenever
/// the tank *possesses* the weapon, regardless of whether laser currently
/// outranks it in `active_weapon()` - the mount is a physical object on the
/// turret, not a firing-mode indicator. Independent of hull damage tier
/// (`hull_col`'s light/disabled/wreck ladder): hull and turret art are
/// already fully decoupled layers, and this overlay only checks
/// turret-adjacent state (`is_wreck`), so a hull that's gone
/// light/disabled never hides it.
///
/// Drawn at the exact same `dest`/`origin` as `draw_tank`'s hull/turret
/// layers (same shared pivot, see `draw_pivot`) - the overlay's own art
/// (`tools/spritegen/gen_minigun_mount.py`) is authored around that
/// identical cell-center-ish pivot, with its barrels extending forward from
/// it by a fixed on-canvas distance exactly like `gen_tanks.py` already
/// draws every turret's own barrel rects from that same pivot outward - so
/// it drops into place with no offset math, exactly like the turret layer
/// itself. Positioning it instead via the muzzle-offset formula
/// (`TANK_MUZZLE_FORWARD_OFFSET_BY_ROW`, which uses the tank's instant,
/// snapped `rotation`) would visibly detach the cluster from the turret
/// mid-turn, since this overlay rotates at the eased
/// `turret_visual_rotation` instead - that formula stays reserved for what
/// it's proven for: positioning where `Bullet`s actually spawn.
///
/// Rotated by `turret_visual_rotation` ONLY (to track the turret's own
/// eased aim) - unlike the turret itself, this does NOT add any further
/// spin: `minigun_mount.png` is a 3-column sheet, one "hot barrel" per
/// column (see `minigun_cycle_frame`/`minigun_cycle_timer`), cycled instead
/// of rotated. A top-down camera looks edge-on at a barrel cluster's real
/// rotation axis (the barrels point along the ground plane, toward the
/// target), so spinning this sprite in the screen plane would read as a
/// helicopter rotor seen from above - the wrong axis entirely for this
/// camera angle. See `tools/spritegen/gen_minigun_mount.py`'s module doc
/// comment for the full reasoning. Scaled by the flat `MINIGUN_MOUNT_SCALE`
/// (not indexed by chassis row) layered on the tank's own `scale` - the
/// mount is deliberately the same size on every chassis, a fixed piece of
/// hardware rather than something that scales with the tank it's bolted to.
pub fn draw_minigun_mount(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    if tank.minigun_ammo <= 0 || tank.is_wreck() {
        return;
    }
    let src = Rectangle::new(
        tank.minigun_cycle_frame() as f32 * MINIGUN_MOUNT_TEXTURE_SIZE,
        0.0,
        MINIGUN_MOUNT_TEXTURE_SIZE,
        MINIGUN_MOUNT_TEXTURE_SIZE,
    );
    let size = MINIGUN_MOUNT_TEXTURE_SIZE * tank.scale * MINIGUN_MOUNT_SCALE;
    let dest = Rectangle::new(tank.position.x, tank.position.y, size, size);
    let origin = draw_pivot(size);
    d.draw_texture_pro(texture, src, dest, origin, tank.turret_visual_rotation, Color::WHITE);
}

/// Shadow pass for `draw_minigun_mount` - same tint/offset convention as
/// `draw_tank_shadow` (`TANK_SHADOW_OFFSET`/`TANK_SHADOW_OPACITY`, not a
/// separate constant): it's rigidly bolted to the turret, so it should read
/// at the exact same height/offset as the turret's own shadow. Call before
/// `draw_minigun_mount` (and after `draw_tank_shadow`), same ordering rule
/// as every other shadow pass.
pub fn draw_minigun_mount_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    if tank.minigun_ammo <= 0 || tank.is_wreck() {
        return;
    }
    let src = Rectangle::new(
        tank.minigun_cycle_frame() as f32 * MINIGUN_MOUNT_TEXTURE_SIZE,
        0.0,
        MINIGUN_MOUNT_TEXTURE_SIZE,
        MINIGUN_MOUNT_TEXTURE_SIZE,
    );
    let size = MINIGUN_MOUNT_TEXTURE_SIZE * tank.scale * MINIGUN_MOUNT_SCALE;
    let dest = Rectangle::new(
        tank.position.x + tuning().shadow_dir_x * tuning().tank_shadow_offset,
        tank.position.y + tuning().shadow_dir_y * tuning().tank_shadow_offset,
        size,
        size,
    );
    let origin = draw_pivot(size);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().tank_shadow_opacity * tank.alpha()) as u8);
    d.draw_texture_pro(texture, src, dest, origin, tank.turret_visual_rotation, shadow);
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
        let mut tank = Tank { damage: 10.0, shield_timer: 1.0, ..Tank::default() };
        tank.take_damage(30.0, MAX_DAMAGE);
        assert_eq!(tank.damage, 10.0, "shield absorbs the whole hit");
        assert!(!tank.is_wreck());
    }

    #[test]
    fn damage_lands_and_caps_once_the_shield_is_gone() {
        let mut tank = Tank { damage: 10.0, shield_timer: 0.0, ..Tank::default() };
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
    fn shield_charge_is_the_fraction_of_the_duration_left() {
        let duration = tuning().shield_duration_seconds;
        let with = |shield_timer: f32| Tank { shield_timer, ..Tank::default() }.shield_charge();
        assert_eq!(with(duration), 1.0);
        assert!((with(duration / 4.0) - 0.25).abs() < 1e-5);
        assert_eq!(with(duration * 3.0), 1.0, "a timer pushed past the knob still reads as full");
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
        let fade = tuning().shield_glow_fade_seconds;
        assert_eq!(enemy_health_ring_visibility(&Tank { shield_timer: fade * 2.0, ..enemy(60.0, 3.0) }), 0.0);
        assert_eq!(player_health_ring_visibility(&Tank { shield_timer: fade * 2.0, ..Tank::default() }), 0.0);
        let dropping = Tank { shield_timer: fade / 2.0, ..Tank::default() };
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
        tank.ring_velocity = Vector2::new(40.0, 0.0);
        tank.ease_ring_position(1.0 / 60.0);
        assert_eq!(tank.ring_velocity, Vector2::new(0.0, 0.0));
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
        assert_eq!(tank(Owner::Player(1), 11).row, 11);
        assert_eq!(icon_source_rec(0).y, (TANK_ROWS_PER_TEAM as f32) * TANK_TEXTURE_SIZE);
        assert_eq!(icon_source_rec(1).y, (2 * TANK_ROWS_PER_TEAM) as f32 * TANK_TEXTURE_SIZE);
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
