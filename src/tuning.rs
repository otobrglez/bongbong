//! Runtime-tunable game parameters - the live half of what used to be
//! `lib.rs`'s wall of `pub const`s. See docs/runtime-tuning-design.md for
//! the full design; the short version:
//!
//! - Every knob is one row in the [`tunables!`] table below (name, type,
//!   default, allowed range, optional per-variant labels, and when a change
//!   takes effect). The `///` doc comment on a row is the same text the old
//!   constant carried in `lib.rs`, and it's *captured* into [`Tuning::SCHEMA`]
//!   so the dev panel can show it - keep it as the source of truth for what
//!   a knob means, same convention as before.
//! - The table expands to the [`Tuning`] struct (one field per row),
//!   [`Tuning::DEFAULT`], the static [`Tuning::SCHEMA`] table, and
//!   range-checked [`Tuning::get`]/[`Tuning::set`] by name.
//! - Code reads knobs through [`tuning()`], a global read guard:
//!   `tuning().tank_speed`. Bind it once (`let t = tuning();`) in a hot loop.
//! - **Writes only ever happen at the frame boundary.** Every transport (the
//!   `dev-tools` C API in `capi.rs`, `--tuning <file>` + its mtime watch in
//!   `main.rs`) *stages* a new table via [`submit_json`]/[`submit_reset`];
//!   the main loop calls [`apply_pending`] right before `Game::update`, so a
//!   frame never observes two values of one knob and nothing inside
//!   `simulation/` ever mutates tuning. Loading a file at startup uses
//!   [`replace_now`], before the first frame.
//!
//! What is *not* here, and stays a `pub const` in `lib.rs`: anything that
//! describes an asset or a data structure (sprite-atlas columns, texture
//! sizes, `*_VARIANTS`, collider bounding boxes, grid cell sizes, physics
//! timestep). Changing those at runtime would desync sprite slicing, the nav
//! grid, or the map format - they're layout, not tuning.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, RwLock, RwLockReadGuard};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// When a change to a knob is actually felt in play. Purely informational
/// (shown as a badge in the dev panel) - every knob is *stored* live; this
/// says whether the code that consumes it reads it every frame or only
/// once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Applies {
    /// Read fresh every frame / on every new entity or shot.
    Live,
    /// Baked into an entity when it spawns (a physics body's mass, a
    /// wall's health) - affects new spawns and the next restart.
    Spawn,
    /// Consumed by `Game::init` only - needs a round restart.
    Restart,
}

/// Element type of a row (for an array row, the element's type).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    F32,
    F64,
    I32,
    U32,
    Usize,
    Bool,
}

/// One row of [`Tuning::SCHEMA`]: everything the dev panel needs to render
/// and validate a knob without knowing anything else about the game.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct ParamMeta {
    /// Field name, `snake_case` - also the JSON key.
    pub name: &'static str,
    /// Which `group { ... }` block the row sits in - a UI tab.
    pub group: &'static str,
    /// Element type.
    pub kind: Kind,
    /// Rust type as written in the table (`f32`, `[f32; 12]`), for the
    /// "Copy as Rust" export.
    pub ty: &'static str,
    /// The row's `///` doc comment, lines joined with `\n`.
    pub doc: &'static str,
    /// Inclusive allowed range, applied to every element of an array row.
    pub min: f64,
    pub max: f64,
    pub applies: Applies,
    /// Empty for a scalar row; one label per element for an array row, in
    /// element order (the indexing enum's declaration order).
    pub labels: &'static [&'static str],
}

impl ParamMeta {
    /// Whether this row is an array (has labels) rather than a scalar.
    pub fn is_array(&self) -> bool {
        !self.labels.is_empty()
    }
}

/// Scalar element types a row may use. Everything round-trips through `f64`
/// so one JSON number type covers every row.
pub trait Knob: Copy {
    const KIND: Kind;
    fn as_f64(self) -> f64;
    fn from_f64(v: f64) -> Self;
}

macro_rules! knob_impl {
    ($($ty:ty => $kind:ident),* $(,)?) => { $(
        impl Knob for $ty {
            const KIND: Kind = Kind::$kind;
            fn as_f64(self) -> f64 { self as f64 }
            fn from_f64(v: f64) -> Self { v as $ty }
        }
    )* };
}
knob_impl!(f32 => F32, f64 => F64, i32 => I32, u32 => U32, usize => Usize);

impl Knob for bool {
    const KIND: Kind = Kind::Bool;
    fn as_f64(self) -> f64 {
        if self { 1.0 } else { 0.0 }
    }
    fn from_f64(v: f64) -> Self {
        v != 0.0
    }
}

/// A row's declared type: either a [`Knob`] scalar or a `[Knob; N]` array.
pub trait Row {
    const KIND: Kind;
}
impl<T: Knob> Row for T {
    const KIND: Kind = T::KIND;
}
impl<T: Knob, const N: usize> Row for [T; N] {
    const KIND: Kind = T::KIND;
}

/// The twelve chassis rows of scifi_tanks_sheet.png, by name, in row order
/// (see docs/SPRITESHEET_SPEC.md §4). Labels every `[_; 12]` row in the
/// table below, so the dev panel can pivot them into one "tank models"
/// grid, and matches `main.rs`'s `--tank` names.
pub const TANK_NAMES: [&str; 12] = [
    "scout",
    "assault",
    "breaker",
    "longbow",
    "flak",
    "wraith",
    "warden",
    "ravager",
    "glacier",
    "obelisk",
    "titan",
    "leviathan",
];

/// Wall materials in `obstacle::MATERIALS` / `obstacle::Material`
/// declaration order - index with `material as usize`.
pub const MATERIAL_NAMES: [&str; 4] = ["brick", "iron", "wood", "glass"];

/// The table macro. Grammar, one row per knob:
///
/// ```text
/// group <name> {
///     /// doc (captured into the schema)
///     <field>: <ty> = <default> in <min> ..= <max> [labels <NAMES>] [@ Live|Spawn|Restart];
/// }
/// ```
///
/// `<default>` is one token tree: a literal for a scalar row, or a bracketed
/// list for an array row (whose element type must be a [`Knob`] and whose
/// length must match `<NAMES>`). A negative scalar default has to be
/// parenthesised, `(-1.0)`, since `-1.0` is two tokens. `@ Live` is the
/// default when the marker is omitted.
macro_rules! tunables {
    ( $( group $group:ident { $(
        $(#[doc = $doc:literal])*
        $name:ident : $ty:ty = $default:tt in $min:literal ..= $max:literal $(labels $labels:ident)? $(@ $applies:ident)? ;
    )* } )* ) => {
        /// Every runtime-tunable knob - see the module docs and
        /// docs/runtime-tuning-design.md. Generated by `tunables!`; the
        /// per-field docs are the table's own.
        #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct Tuning {
            $( $( $(#[doc = $doc])* pub $name: $ty, )* )*
        }

        impl Default for Tuning {
            fn default() -> Self {
                Self::DEFAULT
            }
        }

        impl Tuning {
            /// The shipped values - what `lib.rs` used to hard-code.
            pub const DEFAULT: Tuning = Tuning {
                $( $( $name: $default, )* )*
            };

            /// One entry per row, in declaration order: the table the dev
            /// panel renders.
            pub const SCHEMA: &'static [ParamMeta] = &[
                $( $( ParamMeta {
                    name: stringify!($name),
                    group: stringify!($group),
                    kind: <$ty as Row>::KIND,
                    ty: stringify!($ty),
                    doc: concat!($($doc, "\n"),*),
                    min: $min as f64,
                    max: $max as f64,
                    applies: tunables!(@applies $($applies)?),
                    labels: tunables!(@labels $($labels)?),
                }, )* )*
            ];

            /// Read one value by key: `name` for a scalar row, `name.label`
            /// or `name.<index>` for one element of an array row.
            pub fn get(&self, key: &str) -> Option<f64> {
                let (name, elem) = split_key(key);
                match name {
                    $( $( stringify!($name) => tunables!(@load self.$name, elem $(, $labels)?), )* )*
                    _ => None,
                }
            }

            /// Write one value by key (same key forms as `get`), range-checked
            /// against the row's `min ..= max`. Errors name the key.
            pub fn set(&mut self, key: &str, v: f64) -> Result<(), String> {
                let (name, elem) = split_key(key);
                if !v.is_finite() {
                    return Err(format!("{key}: value must be finite"));
                }
                match name {
                    $( $( stringify!($name) => {
                        tunables!(@store self.$name, elem, v, key, $min, $max $(, $labels)?)
                    } )* )*
                    _ => Err(format!("unknown tunable {name:?}")),
                }
            }
        }
    };
    (@applies) => { Applies::Live };
    (@applies $a:ident) => { Applies::$a };
    (@labels) => { &[] };
    (@labels $l:ident) => { &$l };
    // Scalar row.
    (@load $field:expr, $elem:expr) => {
        if $elem.is_some() { None } else { Some(Knob::as_f64($field)) }
    };
    (@store $field:expr, $elem:expr, $v:expr, $key:expr, $min:literal, $max:literal) => {{
        if $elem.is_some() {
            return Err(format!("{} is a scalar, not an array", $key));
        }
        if !($min as f64..=$max as f64).contains(&$v) {
            return Err(format!("{}={} outside {}..={}", $key, $v, $min, $max));
        }
        $field = Knob::from_f64($v);
        Ok(())
    }};
    // Array row.
    (@load $field:expr, $elem:expr, $labels:ident) => {
        $elem.and_then(|e| element_index(&$labels, e)).map(|i| Knob::as_f64($field[i]))
    };
    (@store $field:expr, $elem:expr, $v:expr, $key:expr, $min:literal, $max:literal, $labels:ident) => {{
        let Some(e) = $elem else {
            return Err(format!("{} is an array; address one element as {}.<label>", $key, $key));
        };
        let Some(i) = element_index(&$labels, e) else {
            return Err(format!("{}: no element {:?} (labels: {})", $key, e, $labels.join(", ")));
        };
        if !($min as f64..=$max as f64).contains(&$v) {
            return Err(format!("{}={} outside {}..={}", $key, $v, $min, $max));
        }
        $field[i] = Knob::from_f64($v);
        Ok(())
    }};
}

/// `"name.elem"` -> `("name", Some("elem"))`, `"name"` -> `("name", None)`.
fn split_key(key: &str) -> (&str, Option<&str>) {
    match key.split_once('.') {
        Some((n, e)) => (n, Some(e)),
        None => (key, None),
    }
}

/// Resolve an array element by label (`"titan"`) or by index (`"10"`).
fn element_index(labels: &[&str], elem: &str) -> Option<usize> {
    labels
        .iter()
        .position(|l| *l == elem)
        .or_else(|| elem.parse::<usize>().ok().filter(|i| *i < labels.len()))
}

tunables! {
    group round {
        /// Number of enemy tanks is randomized within this range each round
        /// (overridden by `--enemies`/the map's own `tanks` count).
        enemy_count_min: usize = 3 in 0 ..= 30 @ Restart;
        enemy_count_max: usize = 10 in 0 ..= 30 @ Restart;
        /// Enemies spawn in a band that's between these fractions of the
        /// shorter screen dimension away from the nearest edge of the
        /// battlefield - close enough to feel like they're closing in from
        /// the sides, but never right on the edge or dropped in the middle.
        enemy_spawn_margin_min: f32 = 0.272 in 0.0 ..= 0.5 @ Restart;
        enemy_spawn_margin_max: f32 = 0.4 in 0.0 ..= 0.5 @ Restart;
        /// Fraction of enemies that start each round already carrying a
        /// special weapon (laser, plasma, or minigun - see the two shares
        /// below) loaded with a full pickup's worth of ammo, rather than the
        /// shell-only default every tank otherwise spawns with. Rolled
        /// independently per enemy, so this is an expected fraction across
        /// a round, not an exact headcount.
        enemy_special_weapon_chance: f32 = 0.268 in 0.0 ..= 1.0 @ Restart;
        /// Of an enemy that rolls a special weapon, the odds it's a laser.
        enemy_special_weapon_laser_share: f32 = 0.5 in 0.0 ..= 1.0 @ Restart;
        /// Of the remaining (non-laser) share, the odds it's plasma rather
        /// than a minigun. 0.5 means the two split the non-laser half evenly
        /// (laser 50%, plasma 25%, minigun 25% overall).
        enemy_special_weapon_plasma_share: f32 = 0.5 in 0.0 ..= 1.0 @ Restart;
        /// When the round ends (player destroyed, or all enemies destroyed)
        /// the result is shown for this long, then the game restarts.
        restart_delay: f32 = 3.0 in 0.0 ..= 30.0;
    }

    group movement {
        /// Player top speed (px/s). Tank driving is 4-direction movement
        /// with real inertia, modeled like a tracked vehicle rather than a
        /// car: pressing a direction snaps the hull facing immediately and
        /// the velocity along that axis chases the commanded speed via a
        /// mass-aware acceleration impulse each frame (see
        /// `Game::drive_tank`, `tank_accel_force`/`tank_decel_curve_rate`).
        tank_speed: f32 = 210.0 in 20.0 ..= 800.0;
        /// Baseline enemy top speed (px/s), slower than the player. Each
        /// enemy's own top speed is this times its spawn-rolled
        /// `Tank::speed_scale` (see `enemy_speed_variance`).
        enemy_speed: f32 = 160.0 in 20.0 ..= 800.0;
        /// Each enemy's speed is randomized within +/- this fraction of
        /// `enemy_speed` at spawn, so some drive faster and some slower
        /// instead of all moving in lockstep.
        enemy_speed_variance: f32 = 0.25 in 0.0 ..= 1.0 @ Spawn;
        /// A damaged tank slows down: both its top speed and how fast it can
        /// reach it are scaled by a curve of how hurt it is (0 = pristine,
        /// 1 = about to wreck; see `Tank::speed_factor`). The curve stays
        /// close to full effect through light and moderate damage, then
        /// falls off harder as damage climbs toward the max - a limp rather
        /// than a straight-line taper - bottoming out at this floor instead
        /// of zero (a tank stops moving separately, once it's a wreck).
        damage_speed_floor: f32 = 0.35 in 0.0 ..= 1.0;
        /// Exponent of the damage-slowdown curve above (higher = holds full
        /// speed longer, then drops harder).
        damage_speed_curve: f32 = 2.2 in 0.1 ..= 10.0;
        /// Acceleration: how fast a tank's actual velocity can chase its
        /// commanded target, expressed as a force so mass genuinely matters
        /// (F = m*a - a heavier tank ramps slower for the same force) rather
        /// than a flat px/s^2 every tank shares. Also scaled by
        /// `Tank::speed_factor`, so a damaged tank is sluggish to speed up,
        /// not just capped at a lower top speed. Kept meaningfully
        /// *stronger* than `tank_turn_grip_force` so a 90-degree turn
        /// doesn't stall: the new axis's speed builds faster than the old
        /// axis's momentum decays, so speed through a corner stays near top
        /// speed instead of bottoming out. Reaches `tank_speed` in well
        /// under a second.
        tank_accel_force: f32 = 4200.0 in 100.0 ..= 30000.0;
        /// Braking curve (releasing a direction, reversing, or coasting off
        /// a knockback): `1 - exp(-rate * dt)` is the fraction of the
        /// remaining speed gap closed this frame, frame-rate independent. A
        /// curve by construction - bites hardest right when a direction is
        /// released and tapers as the tank nears a stop. Divides by
        /// `Tank::mass` like accel; a `std`-class tank reads as "most of the
        /// way stopped" in ~150ms at 80. Higher = snappier stop. Verify
        /// headlessly with `cargo run --bin probe -- --scenario brake`.
        tank_decel_curve_rate: f32 = 80.0 in 1.0 ..= 500.0;
        /// The exponential brake only approaches zero asymptotically, so
        /// this is the remaining speed-gap (px/s) below which
        /// `Game::drive_tank` snaps straight to the target instead of
        /// trailing an imperceptible tail forever.
        tank_decel_snap_px: f32 = 4.0 in 0.0 ..= 50.0;
        /// Turning grip: how hard a tank's tracks cancel velocity
        /// *perpendicular* to the hull's current facing - the "traction"
        /// knob. Deliberately *weaker* than `tank_accel_force`: full
        /// snap-to-new-axis read as too clinically controlled through a
        /// corner, so the old axis's momentum now visibly carries through a
        /// turn as a genuine drift (~100ms scrub-to-zero at top speed at
        /// 2200). Not scaled by `Tank::speed_factor` (track grip is a
        /// mechanical property, not engine power). Applies every frame
        /// whether or not a direction is held, so sideways knockback gets
        /// scrubbed by this too.
        tank_turn_grip_force: f32 = 2000.0 in 0.0 ..= 30000.0;
        /// Purely cosmetic hull-turn animation: `Tank::rotation` itself
        /// still snaps instantly (physics/aim/track heading all key off it);
        /// `Tank::visual_rotation` chases it at this many degrees per second
        /// (shortest way round), so a corner visibly swings the hull over a
        /// few frames rather than popping to the new facing.
        tank_visual_turn_speed_deg: f32 = 720.0 in 30.0 ..= 3600.0;
        /// The turret eases toward the same commanded rotation independently
        /// of the hull, at its own (faster) turn speed, so it visibly leads
        /// a turn while the heavier hull swings around to catch up.
        tank_turret_visual_turn_speed_deg: f32 = 1800.0 in 30.0 ..= 7200.0;
    }

    group shell {
        /// Shell flight speed (px/s).
        shell_speed: f32 = 500.0 in 50.0 ..= 3000.0;
        /// Half-extent (px) of a shell's own hit box, inflating every target
        /// box the swept hit test (`simulation::hits::Terrain::sweep`)
        /// checks its flight segment against. Kept small and near-point-like
        /// so a hit still reads as "did the shell's position land inside the
        /// tank" rather than "did two boxes overlap". Also the laser beam's
        /// half-width.
        shell_hit_half_extent: f32 = 3.0 in 0.5 ..= 32.0;
        /// Shell ammo: a tank holds up to this many shells (its magazine at
        /// spawn, and the passive-recharge cap) - a pickup is the only way
        /// past it.
        max_shells: i32 = 12 in 1 ..= 100 @ Spawn;
        /// Recharge one shell every this many seconds while below
        /// `max_shells`.
        shell_recharge_seconds: f32 = 2.0 in 0.05 ..= 30.0;
        /// Player fire rate: minimum seconds between consecutive player
        /// shots (`Tank::fire_cooldown`) - without it, holding fire could
        /// dump the whole magazine in a few frames. The AI has its own
        /// `enemy_fire_interval` gate instead.
        player_fire_interval: f32 = 0.15 in 0.0 ..= 5.0;
        /// How long after a twin-barrel chassis's first shell the second one
        /// fires (`Tank::pending_shot`) - long enough to read as two shots,
        /// short enough that it's still clearly one trigger-pull. Keep it
        /// under `player_fire_interval` and the AI's fastest interval so the
        /// pending second shell resolves before that tank could fire again.
        tank_twin_shot_delay_seconds: f32 = 0.05 in 0.0 ..= 1.0;
        /// Player shell damage, rolled uniformly in this range per hit and
        /// then scaled by the shooter's `tank_damage_factor`.
        player_damage_min: f32 = 10.0 in 0.0 ..= 100.0;
        player_damage_max: f32 = 30.0 in 0.0 ..= 100.0;
        /// Enemy shell damage range (weaker than the player's).
        enemy_damage_min: f32 = 5.0 in 0.0 ..= 100.0;
        enemy_damage_max: f32 = 15.0 in 0.0 ..= 100.0;
        /// Firing recoil: a small backward impulse on the shooter along the
        /// shell's own travel axis, mass-normalized so a heavier chassis
        /// visibly recoils less per shot. Deliberately much smaller than
        /// `knockback_max_speed`: felt, not a real shove.
        shell_recoil_speed: f32 = 18.0 in 0.0 ..= 200.0;
        shell_recoil_max_speed: f32 = 40.0 in 0.0 ..= 400.0;
        /// A shell impact gives the tank it hits a small shove along the
        /// shell's travel direction - a "tap", not a shove - skipped if this
        /// very hit just wrecked the tank.
        shell_impact_knockback_speed: f32 = 35.0 in 0.0 ..= 400.0;
        /// Ricochet: a shell reflects off an indestructible Iron obstacle
        /// instead of detonating, up to this many times per shell. Every
        /// other target (a tank, the frog, the boundary wall, any
        /// destructible material) still detonates on first contact. One
        /// keeps the Iron case readable: a grazing shot gets one more chance
        /// to land, not an indefinitely ping-ponging shell.
        shell_ricochet_bounces: u32 = 1 in 0 ..= 10;
    }

    group minigun {
        /// Rounds granted per minigun pickup (~5 full bursts at 8/burst).
        minigun_ammo_per_pickup: i32 = 40 in 1 ..= 1000;
        /// Bullets per burst: the first fires on the trigger frame, the rest
        /// are queued `minigun_bullet_delay_seconds` apart
        /// (`Tank::minigun_burst`). Each is an individually simulated
        /// `Bullet`, not one abstract burst object.
        minigun_burst_size: u32 = 6 in 1 ..= 64;
        /// Gap between successive bullets of one burst. In
        /// `tank_twin_shot_delay_seconds`'s neighborhood, a touch tighter so
        /// the stutter reads busier than a twin-cannon's second shot.
        minigun_bullet_delay_seconds: f32 = 0.04 in 0.005 ..= 1.0;
        /// Extra gap held after a burst's last bullet before
        /// `Tank::fire_cooldown` clears, so a fresh trigger pulse can't
        /// stack a new burst on an unfinished one.
        minigun_burst_trailing_gap_seconds: f32 = 0.1 in 0.0 ..= 2.0;
        /// Each bullet's direction is jittered by up to this many degrees
        /// off the aim line, stacked on top of any point-blank misfire skew
        /// - the minigun's own "spray" identity.
        minigun_bullet_spread_deg: f32 = 4.0 in 0.0 ..= 90.0;
        /// Bullet flight speed (px/s) - faster than a shell: a zippy tracer,
        /// not a lobbed shell.
        minigun_bullet_speed: f32 = 570.0 in 50.0 ..= 5000.0;
        /// Bullet hit-box half-extent (px) - smaller than a shell's, a
        /// lighter caliber.
        minigun_bullet_hit_half_extent: f32 = 2.0 in 0.5 ..= 32.0;
        /// Per-bullet damage, deliberately well below a laser or shell hit -
        /// a single round is a non-event; a fully-landed burst lands near
        /// one solid shell hit. One shared range for player and enemy,
        /// scaled only by `tank_damage_factor`.
        minigun_bullet_damage_min: f32 = 3.0 in 0.0 ..= 100.0;
        minigun_bullet_damage_max: f32 = 6.0 in 0.0 ..= 100.0;
        /// Per-bullet recoil, much lighter than a shell's - a burst should
        /// rattle the tank, not shove it once hard.
        minigun_bullet_recoil_speed: f32 = 3.0 in 0.0 ..= 200.0;
        minigun_bullet_recoil_max_speed: f32 = 10.0 in 0.0 ..= 400.0;
        /// How long each of minigun_mount.png's 3 "hot barrel" frames is
        /// shown before advancing, while a burst is active (a discrete frame
        /// swap, not a rotation - see `tank::draw_minigun_mount`). Tuned
        /// close to `minigun_bullet_delay_seconds` so roughly one barrel
        /// swap happens per bullet.
        minigun_cycle_seconds: f32 = 0.05 in 0.01 ..= 1.0;
    }

    group laser {
        /// Laser charges granted per pickup. While `Tank::laser_charges > 0`
        /// and the laser is the live weapon, firing resolves an instant hit
        /// the same frame (no travel time) and consumes one charge.
        laser_charges_per_pickup: i32 = 6 in 1 ..= 200;
        /// Per-hit damage range - lower than a player shell's, since a laser
        /// never misses; toned down to compensate for guaranteed accuracy.
        laser_damage_min: f32 = 8.0 in 0.0 ..= 100.0;
        laser_damage_max: f32 = 14.0 in 0.0 ..= 100.0;
        /// Odds a laser pickup rolls the Blue variant (Red is the rest),
        /// rolled once per pickup rather than per shot.
        laser_blue_pickup_chance: f32 = 0.4 in 0.0 ..= 1.0;
        /// A Blue charge batch fires at the damage range above scaled by
        /// this factor instead of the Red baseline (1.0).
        laser_blue_damage_factor: f32 = 1.2 in 0.1 ..= 5.0;
        /// How long a fired beam stays on screen before fading out - purely
        /// cosmetic, a flash that doesn't linger.
        laser_beam_display_seconds: f32 = 0.12 in 0.01 ..= 2.0;
        /// Line thickness (px) of the beam's glow pass - the core pass draws
        /// thinner, at a fixed fraction of this.
        laser_beam_width: f32 = 4.0 in 1.0 ..= 32.0;
    }

    group plasma {
        /// Plasma bolts granted per pickup: 10 single shots, or 5
        /// twin-barrel volleys (a twin chassis spends 2 per shot like a
        /// shell does).
        plasma_ammo_per_pickup: i32 = 10 in 1 ..= 500;
        /// Flat damage multiplier on top of the shell damage range the
        /// shooter would otherwise use (player/enemy split included) and
        /// `tank_damage_factor` - a straight damage upgrade over a shell.
        plasma_damage_factor: f32 = 1.24 in 0.1 ..= 5.0;
        /// Bolt flight speed (px/s) - a touch faster than a shell.
        plasma_speed: f32 = 504.0 in 50.0 ..= 3000.0;
        /// Bolt hit-box half-extent (px) - a fatter bolt is easier to land,
        /// matching its bigger on-screen size.
        plasma_hit_half_extent: f32 = 5.0 in 0.5 ..= 32.0;
        /// Launch recoil - a bit more than a shell's, a heavier kick.
        plasma_recoil_speed: f32 = 22.0 in 0.0 ..= 200.0;
        plasma_recoil_max_speed: f32 = 45.0 in 0.0 ..= 400.0;
        /// Impact shove on the tank hit - a heavier "tap" than a shell's.
        plasma_impact_knockback_speed: f32 = 45.0 in 0.0 ..= 400.0;
        /// Odds a plasma pickup rolls the Purple variant (Teal is the base),
        /// rolled once per pickup.
        plasma_purple_pickup_chance: f32 = 0.3 in 0.0 ..= 1.0;
        /// A Purple charge batch scales `plasma_damage_factor` by this on
        /// top.
        plasma_purple_damage_factor: f32 = 1.10 in 0.1 ..= 5.0;
        /// Pulsating in-flight glow (`plasma::draw_plasma`): pulses per
        /// second, and the glow radius at the low/high point of the pulse as
        /// a multiple of the sprite radius.
        plasma_pulse_hz: f32 = 6.0 in 0.1 ..= 30.0;
        plasma_pulse_min_scale: f32 = 0.85 in 0.1 ..= 3.0;
        plasma_pulse_max_scale: f32 = 1.35 in 0.1 ..= 3.0;
        /// The Flying state's baked 4-frame breathing cycle plays this many
        /// full cycles per second, independent of `plasma_pulse_hz` - two
        /// independent cycles read richer than one rate driving both.
        plasma_flying_cycle_fps: f32 = 10.0 in 0.5 ..= 60.0;
    }

    group pickups {
        /// Seconds after a pickup is collected before a fresh one spawns at
        /// a random empty map slot - keeps the field topped up.
        pickup_respawn_seconds: f32 = 15.0 in 0.0 ..= 120.0;
        /// How close a tank's center needs to get to collect a pickup - more
        /// forgiving than true hull overlap.
        pickup_collect_radius: f32 = 32.0 in 4.0 ..= 200.0;
        /// Health pickup: deliberately not a full heal - roughly 2-3 enemy
        /// hits' worth, worth detouring for.
        pickup_heal_amount: f32 = 40.0 in 0.0 ..= 100.0;
        /// Ammo pickup: how many shells it adds. Uncapped - the only way past
        /// `max_shells`, and stacking pickups can push a magazine
        /// arbitrarily high.
        pickup_ammo_amount: i32 = 10 in 0 ..= 100;
        /// Speed-up pickup: `Tank::effective_speed` is scaled by this while
        /// the boost is active.
        speed_boost_multiplier: f32 = 1.3 in 1.0 ..= 4.0;
        /// Collecting a speed-up *sets* the boost timer to this (a second
        /// one refreshes it rather than stacking) - a tank is only ever
        /// under one boost at a time.
        speed_boost_duration_seconds: f32 = 12.0 in 0.0 ..= 120.0;
    }

    group combat {
        /// Ramming: after taking collision damage a tank is immune for this
        /// long, so continuous touching doesn't drain damage every frame.
        ram_damage_cooldown: f32 = 0.5 in 0.0 ..= 5.0;
        /// Damage both tanks take from one ram contact, rolled uniformly in
        /// this range (`simulation::combat::ram`). Wrecks neither deal nor
        /// take it.
        ram_damage_min: f32 = 2.0 in 0.0 ..= 100.0;
        ram_damage_max: f32 = 6.0 in 0.0 ..= 100.0;
        /// A ram also gives both tanks a brief knockback shove apart: this
        /// fraction of the closing speed becomes push speed, mass-normalized
        /// so a lighter tank gets shoved further. Wrecks are infinite mass.
        knockback_strength: f32 = 0.2 in 0.0 ..= 2.0;
        /// px/s cap on any one ram push - keeps it small.
        knockback_max_speed: f32 = 60.0 in 0.0 ..= 500.0;
        /// A tank's death deals a small splash of damage to opposing tanks
        /// within this radius (px), on top of the shockwave visual.
        explosion_radius: f32 = 110.0 in 0.0 ..= 600.0;
        /// Splash damage range - a chip, not a second kill shot; never chips
        /// the dead tank's own side.
        explosion_damage_min: f32 = 3.0 in 0.0 ..= 100.0;
        explosion_damage_max: f32 = 8.0 in 0.0 ..= 100.0;
        /// The explosion's outward shove isn't side-restricted: every live
        /// tank in range gets pushed, full at ground zero tapering linearly
        /// to nothing at `explosion_radius`.
        explosion_knockback_speed: f32 = 90.0 in 0.0 ..= 500.0;
        /// A wreck burns for this long, then settles into a static charred
        /// hulk.
        wreck_burn_seconds: f32 = 4.0 in 0.0 ..= 30.0;
    }

    group ai {
        /// Start chasing the player within this distance (px). Was 520; at
        /// that value enemies never noticed the player past roughly half the
        /// window and read as passive.
        enemy_view_range: f32 = 800.0 in 50.0 ..= 3000.0;
        /// Stop and fight within this distance (px). The engagement ring and
        /// retreat range are factors of this - see the `engage` group.
        enemy_attack_range: f32 = 340.0 in 50.0 ..= 2000.0;
        /// Fire when the player is within this many px of the firing axis.
        enemy_fire_align_px: f32 = 24.0 in 1.0 ..= 200.0;
        /// Minimum seconds between AI shots at the baseline magazine level.
        enemy_fire_interval: f32 = 1.2 in 0.05 ..= 10.0;
        /// The fuller an enemy's magazine, the faster it re-fires: at
        /// `max_shells` it uses this interval instead; ammo between
        /// `enemy_ammo_low` and `max_shells` interpolates linearly.
        enemy_fire_interval_aggressive: f32 = 0.7 in 0.05 ..= 10.0;
        /// Must be lined up on the player this long before firing.
        enemy_aim_settle: f32 = 0.25 in 0.0 ..= 5.0;
        /// Retreat toward the map edge once this hurt.
        enemy_flee_damage: f32 = 70.0 in 0.0 ..= 100.0;
        /// How often patrol picks a new wander point (seconds).
        enemy_retarget_seconds: f32 = 3.0 in 0.1 ..= 30.0;
        /// How many candidate waypoints `Ai::wander` rolls per resample,
        /// keeping whichever is both reachable and farthest from every other
        /// live tank - plain uniform sampling kept landing wanderers in the
        /// same small reachable pocket. Each candidate costs one grid
        /// pathfind check.
        wander_spread_candidates: u32 = 6 in 1 ..= 32;
        /// Shared aggression: once any enemy sees the player, every enemy
        /// converges on that last known position for this many seconds after
        /// the last sighting (refreshed while it holds).
        enemy_alert_hold_seconds: f32 = 6.0 in 0.0 ..= 60.0;
        /// Retaliation: a hit enemy treats the player as in view for this
        /// long (`Ai::notify_hit`), per tank - shooting one makes *it* fight
        /// back, not the whole field.
        enemy_hit_alert_seconds: f32 = 6.0 in 0.0 ..= 60.0;
        /// Ammo-aware aggression: back off (without firing) once ammo drops
        /// to/below this, ...
        enemy_ammo_low: i32 = 2 in 0 ..= 100;
        /// ... and only re-engage once recharged back up to this. Kept apart
        /// so the enemy doesn't flicker between retreating and attacking.
        enemy_ammo_resume: i32 = 5 in 0 ..= 100;
        /// Friendly-fire avoidance: when a teammate sits on the firing line
        /// closer than the player, the chance the enemy holds fire - not a
        /// hard block, so stray friendly fire still happens.
        enemy_friendly_fire_hold_chance: f32 = 0.6 in 0.0 ..= 1.0;
        /// Point-blank misfires: within this distance of the player an
        /// enemy's shot may be thrown off its aim so it sails wide.
        enemy_misfire_range: f32 = 180.0 in 0.0 ..= 1000.0;
        /// Misfire odds right on top of the player (zero at
        /// `enemy_misfire_range`, scaling up the closer it is).
        enemy_misfire_chance_max: f32 = 0.6 in 0.0 ..= 1.0;
        /// A misfire deflects the shell by a random angle in this range
        /// (degrees).
        enemy_misfire_angle_min: f32 = 12.0 in 0.0 ..= 180.0;
        enemy_misfire_angle_max: f32 = 35.0 in 0.0 ..= 180.0;
        /// Predictive collision avoidance (`Ai::avoid_collisions`): seconds
        /// ahead to predict the closest approach to every other tank.
        avoid_lookahead: f32 = 0.8 in 0.0 ..= 5.0;
        /// Extra clearance beyond the two hull radii (px) before a sidestep
        /// triggers.
        avoid_margin: f32 = 12.0 in 0.0 ..= 100.0;
        /// How long a sidestep is held once triggered.
        avoid_dodge_seconds: f32 = 0.4 in 0.0 ..= 5.0;
        /// Skip prediction when moving slower than this (px/s).
        avoid_min_speed: f32 = 10.0 in 0.0 ..= 200.0;
        /// Direction commitment: once an AI picks a cardinal heading it
        /// holds it for at least this long ...
        ai_dir_hold_seconds: f32 = 0.35 in 0.0 ..= 5.0;
        /// ... and only switches to a new heading that beats the current one
        /// by this margin (px). Together these stop frame-to-frame jitter
        /// near 45-degree diagonals.
        ai_dir_switch_margin_px: f32 = 20.0 in 0.0 ..= 200.0;
        /// A committed heading about to walk into a known-blocked grid cell
        /// can be overridden, but only after this much dwell time - much
        /// shorter than `ai_dir_hold_seconds`, yet without some floor a
        /// coarse grid's routed direction flip-flops every frame near a
        /// corner (found via the probe's `--rounds` sweep).
        ai_obstacle_override_hold_seconds: f32 = 0.1 in 0.0 ..= 2.0;
        /// Stuck-escape: a tank commanded to move whose real physics speed
        /// stays under this (px/s) ...
        stuck_speed_eps: f32 = 8.0 in 0.0 ..= 100.0;
        /// ... for this many seconds running is treated as genuinely stuck,
        /// and `Ai::steer` forces a hard perpendicular-turn reset.
        stuck_escape_seconds: f32 = 0.75 in 0.05 ..= 10.0;
    }

    group engage {
        /// Engagement ring radius as a fraction of `enemy_attack_range`
        /// (`Tuning::engage_ring_radius`): each engaged enemy claims a
        /// distinct slot on one of 4 cardinal axes through the player at
        /// this distance, with per-frame mutual exclusion, so a group
        /// converging on the player doesn't pile up on one point (the real
        /// cause of "clustering"). Comfortably inside attack range so a
        /// firing-line enemy still ends up close enough to fight.
        engage_ring_factor: f32 = 0.8 in 0.1 ..= 1.0;
        /// Lateral offset (px, perpendicular to the axis) between the two
        /// rank-0 firing slots on the same axis. Kept under
        /// `enemy_fire_align_px` so both slots stay inside the alignment
        /// band, while separating the pair by double this - clear of every
        /// hull width, so a paired teammate doesn't read as blocking the
        /// shot.
        engage_lateral_offset: f32 = 18.0 in 0.0 ..= 100.0;
        /// The reserve rank (an axis's 3rd/4th tank) sits this many px past
        /// `enemy_attack_range` (`Tuning::engage_reserve_radius`) - so it
        /// neither fires nor blocks a lane, while staying inside view range
        /// so Chase keeps steering it there.
        engage_reserve_extra_px: f32 = 60.0 in 0.0 ..= 500.0;
        /// The shortest forward distance a rank-0 slot may be clamped down
        /// to when a near-wall player would otherwise push it off the
        /// battlefield - below this the slot is invalid. Just past
        /// `enemy_misfire_range` so a clamped slot never lands in the
        /// forced-misfire zone.
        engage_min_radius: f32 = 190.0 in 0.0 ..= 1000.0;
        /// While retreating on low ammo, back off only to this multiple of
        /// `enemy_attack_range` (`Tuning::enemy_retreat_range`), not all the
        /// way to the map edge like the health-based flee does.
        enemy_retreat_range_factor: f32 = 1.3 in 1.0 ..= 5.0;
    }

    group frog {
        /// The frog's health - deliberately much lower than a tank's 100, a
        /// couple of hits end the round, so "protect the frog" is a real
        /// constraint on where the player fights.
        frog_max_health: f32 = 40.0 in 1.0 ..= 500.0 @ Restart;
        /// Spawn placement: kept this far from the player's start - far
        /// enough not to spawn on top of the player, close enough that
        /// protecting it and protecting yourself are the same fight early.
        frog_spawn_min_dist: f32 = 90.0 in 0.0 ..= 1000.0 @ Restart;
        frog_spawn_max_dist: f32 = 240.0 in 0.0 ..= 1000.0 @ Restart;
        /// Evasion hop distance as a factor of the frog's on-screen size.
        /// 0.75 (was 3.0, then 1.5 - a full body-length-times-three read as
        /// an unnaturally huge leap; user feedback, 2026-08).
        frog_hop_distance_factor: f32 = 0.75 in 0.0 ..= 5.0;
        /// Debounce so a rapid volley doesn't trigger a hop every frame -
        /// roughly one hop per this many seconds under sustained fire.
        frog_hop_cooldown_seconds: f32 = 1.0 in 0.0 ..= 10.0;
        /// Random jitter (degrees) on the ideal dead-away-from-the-shot hop
        /// angle so hops don't all look mechanically identical.
        frog_hop_angle_jitter_deg: f32 = 10.0 in 0.0 ..= 90.0;
        /// Hop landing spots must stay this far (px) inside the battlefield
        /// edge.
        frog_hop_bounds_margin: f32 = 40.0 in 0.0 ..= 200.0;
        /// Bite any tank - either side - within this factor of the frog's
        /// size.
        frog_attack_range_factor: f32 = 0.9 in 0.0 ..= 5.0;
        frog_attack_cooldown_seconds: f32 = 1.5 in 0.0 ..= 10.0;
        frog_attack_damage_min: f32 = 4.0 in 0.0 ..= 100.0;
        frog_attack_damage_max: f32 = 10.0 in 0.0 ..= 100.0;
        /// Personal space: hop away from the nearest tank once one gets
        /// within this factor of the frog's size - larger than the bite
        /// range ("keep your distance" rather than "retaliate"), well inside
        /// the hop distance so a single hop reliably clears it.
        frog_avoid_range_factor: f32 = 1.2 in 0.0 ..= 5.0;
    }

    group walls {
        /// Per-material toughness: hp absorbed before reaching the terminal
        /// state (rubble/charred/shattered), or - for Iron - before its rust
        /// stage plateaus (Iron is never destroyed). Ordered fragile to
        /// tough: glass snaps almost immediately, wood breaks easily, brick
        /// holds longer, iron the longest of all on top of being permanent.
        /// Baked into each wall's health when the map is spawned.
        wall_max_health: [f32; 4] = [20.0, 220.0, 10.0, 2.0] in 1.0 ..= 1000.0 labels MATERIAL_NAMES @ Spawn;
        /// Fraction of spawned Wood obstacles that catch fire when destroyed
        /// instead of breaking outright (`Obstacle::flammable`, rolled once
        /// at spawn).
        wood_flammable_chance: f64 = 0.7 in 0.0 ..= 1.0 @ Spawn;
        /// Burning wood's 3-frame flicker loop cadence (~7.7 FPS at 0.13).
        wood_burn_frame_seconds: f32 = 0.395 in 0.01 ..= 2.0;
        /// Total time a Wood obstacle spends burning before charring and
        /// being removed.
        wood_burn_seconds: f32 = 2.5 in 0.1 ..= 30.0;
    }

    group tank_models {
        /// Chassis mass multiplier, from the 7 handling-weight classes
        /// (narrow 12x18: scout/wraith; compact 14x16: flak/glacier; std
        /// 14x20: assault/warden; long 14x24: longbow/obelisk; wide 16x22:
        /// breaker/ravager; super_heavy 22x24: titan; super_long 20x26:
        /// leviathan) - each is that class's footprint area over `std`'s
        /// 280, so `std` is exactly 1.0. Drives `Tank::mass` (accel, drift,
        /// how far a ram shoves it). The physics body's mass is set from it
        /// at spawn.
        tank_mass_factor: [f32; 12] = [216.0 / 280.0, 1.0, 352.0 / 280.0, 336.0 / 280.0, 224.0 / 280.0, 216.0 / 280.0, 1.0, 352.0 / 280.0, 224.0 / 280.0, 336.0 / 280.0, 528.0 / 280.0, 520.0 / 280.0] in 0.1 ..= 5.0 labels TANK_NAMES @ Spawn;
        /// Per-chassis damage multiplier on everything this chassis fires,
        /// on top of the weapon's own range. Same 7 classes as the mass
        /// table but tuned by the sheet's role hints (a sniper platform hits
        /// hard without being the heaviest); `std` is exactly 1.0. Pairs with
        /// mass by design: the super-heavies are the slowest to drive and
        /// get the payoff of hitting hardest; `narrow` is fast, evasive,
        /// weak.
        tank_damage_factor: [f32; 12] = [0.75, 1.0, 1.20, 1.35, 0.90, 0.75, 1.0, 1.20, 0.90, 1.35, 1.55, 1.60] in 0.1 ..= 5.0 labels TANK_NAMES;
        /// How far forward (tile px, toward the hull's facing) a shot spawns
        /// from the tank's center - where the barrel tip actually sits.
        /// From the sprite spec's turret bbox y0 per row (16 - y0).
        tank_muzzle_forward_offset: [f32; 12] = [14.0, 13.0, 12.0, 16.0, 10.0, 13.0, 14.0, 14.0, 14.0, 16.0, 16.0, 16.0] in 0.0 ..= 32.0 labels TANK_NAMES;
        /// Sideways (tile px) distance from center to each barrel for the
        /// five twin-barrel chassis (assault, flak, ravager, obelisk, titan);
        /// zero for every single-barrel row. Positive is the right-hand
        /// barrel; a twin chassis fires one independent shot per barrel.
        tank_barrel_lateral_offset: [f32; 12] = [0.0, 3.0, 0.0, 0.0, 3.0, 0.0, 0.0, 3.0, 0.0, 3.0, 5.0, 0.0] in 0.0 ..= 16.0 labels TANK_NAMES;
        /// Per-chassis tread-mark size multiplier on `track_scale_fraction`
        /// (a titan presses a visibly bigger mark than a scout), from the
        /// sprite spec's intensity-by-chassis table.
        track_weight_scale: [f32; 12] = [0.75, 1.00, 1.20, 1.10, 0.85, 0.75, 1.00, 1.20, 0.85, 1.10, 1.45, 1.35] in 0.0 ..= 3.0 labels TANK_NAMES;
        /// Per-chassis tread-mark opacity multiplier on `track_max_opacity`
        /// - a heavier chassis presses a darker mark, not just a bigger one.
        track_weight_opacity: [f32; 12] = [0.70, 1.00, 1.20, 1.10, 0.82, 0.70, 1.00, 1.20, 0.82, 1.10, 1.50, 1.35] in 0.0 ..= 3.0 labels TANK_NAMES;
    }

    group cosmetics {
        /// World px of travel between hull tread-animation frame advances
        /// (independent of the ground-decal spacing below).
        tank_hull_track_frame_distance: f32 = 8.0 in 1.0 ..= 64.0;
        /// Drop shadows: shared screen-space offset direction (down-right, a
        /// top-down-arcade convention) - only the distance differs per
        /// entity type. See docs/sprite-shadows-design.md.
        shadow_dir_x: f32 = 0.595 in -1.0 ..= 1.0;
        shadow_dir_y: f32 = 0.48 in -1.0 ..= 1.0;
        /// Tank shadow distance (px) - grounded, stays tight to the hull.
        tank_shadow_offset: f32 = 3.0 in 0.0 ..= 20.0;
        tank_shadow_opacity: f32 = 0.486 in 0.0 ..= 1.0;
        /// Shell shadow distance, rolled once per shell at fire time within
        /// this range - the separation is what reads as "airborne", and
        /// different shells reading as flying at different heights beats
        /// every shot looking identical.
        shell_shadow_offset_min: f32 = 9.0 in 0.0 ..= 60.0;
        shell_shadow_offset_max: f32 = 28.4 in 0.0 ..= 60.0;
        shell_shadow_opacity: f32 = 0.30 in 0.0 ..= 1.0;
        /// Minigun bullet shadow distance range - tighter than a shell's, a
        /// smaller, lower round.
        minigun_bullet_shadow_offset_min: f32 = 5.0 in 0.0 ..= 60.0;
        minigun_bullet_shadow_offset_max: f32 = 10.0 in 0.0 ..= 60.0;
        minigun_bullet_shadow_opacity: f32 = 0.30 in 0.0 ..= 1.0;
        /// Plasma bolt shadow distance range.
        plasma_shadow_offset_min: f32 = 10.0 in 0.0 ..= 60.0;
        plasma_shadow_offset_max: f32 = 22.0 in 0.0 ..= 60.0;
        plasma_shadow_opacity: f32 = 0.30 in 0.0 ..= 1.0;
        /// Wall shadow distance (px) and opacity.
        obstacle_shadow_offset: f32 = 3.0 in 0.0 ..= 20.0;
        obstacle_shadow_opacity: f32 = 0.35 in 0.0 ..= 1.0;
        /// Track marks: a tank drops a ground mark every this many px of
        /// travel. Each mark stamps the raw travel heading, so the curve you
        /// see is the tank's actual path - this tunes sampling density.
        track_spacing: f32 = 5.0 in 1.0 ..= 64.0;
        /// Seconds for a mark to fully fade away (trail length is roughly
        /// speed times this).
        track_lifetime: f32 = 0.8 in 0.05 ..= 10.0;
        /// Mark size relative to the tank sprite - smaller and faint, a
        /// subtle impression in the ground rather than a bold sprite.
        track_scale_fraction: f32 = 0.55 in 0.1 ..= 2.0;
        /// Opacity of a fresh mark, before fading.
        track_max_opacity: f32 = 0.21 in 0.0 ..= 1.0;
        /// Per-tank track "distortion": each tank rolls its own wobble
        /// amplitude (degrees, in this range) ...
        track_wobble_amp_min_deg: f32 = 1.5 in 0.0 ..= 45.0;
        track_wobble_amp_max_deg: f32 = 6.0 in 0.0 ..= 45.0;
        /// ... wavelength (px of travel per full side-to-side cycle) ...
        track_wobble_wavelength_min: f32 = 40.0 in 5.0 ..= 500.0;
        track_wobble_wavelength_max: f32 = 120.0 in 5.0 ..= 500.0;
        /// ... and +/- scale jitter once at spawn, reused for every mark it
        /// lays, so a trail reads as one coherent tank-specific tread
        /// pattern instead of per-mark noise.
        track_scale_jitter: f32 = 0.15 in 0.0 ..= 1.0;
        /// Overhead health bar: shown under a tank for this long after it's
        /// hit ...
        health_bar_overhead_seconds: f32 = 3.0 in 0.0 ..= 20.0;
        /// ... fading out over the trailing this-many seconds.
        health_bar_overhead_fade_seconds: f32 = 0.6 in 0.0 ..= 5.0;
        /// HUD numbers (SHELLS/HP) turn orange below this fraction of max
        /// ...
        hud_warn_threshold: f32 = 0.34 in 0.0 ..= 1.0;
        /// ... and red below this. Conservative on purpose: only flag real
        /// trouble.
        hud_critical_threshold: f32 = 0.104 in 0.0 ..= 1.0;
    }

    group fx {
        /// Kill shockwave (shockwave.fs): seconds the effect plays before
        /// clearing.
        shockwave_duration: f32 = 1.18 in 0.05 ..= 5.0;
        /// Ring growth speed, UV units/sec.
        shockwave_speed: f32 = 0.56 in 0.0 ..= 5.0;
        /// Thickness of the distorted band, UV units.
        shockwave_width: f32 = 0.13 in 0.0 ..= 1.0;
        /// How hard the ring bends the image, UV units.
        shockwave_strength: f32 = 0.102 in 0.0 ..= 0.5;
        /// Camera shake on the same kill trigger: duration (much shorter
        /// than the shockwave so it reads as one punchy hit), px offset at
        /// full strength, and radians/sec of the wobble.
        camera_shake_duration: f32 = 0.3 in 0.0 ..= 3.0;
        camera_shake_magnitude: f32 = 10.0 in 0.0 ..= 100.0;
        camera_shake_frequency: f32 = 40.0 in 1.0 ..= 200.0;
        /// Muzzle-flash heat haze (muzzle_flash.fs): a one-sided outward
        /// puff at the barrel. Hits full strength at the leading edge, so
        /// tuned lower than the shockwave for similar visual intensity.
        muzzle_flash_duration: f32 = 0.12 in 0.01 ..= 2.0;
        muzzle_flash_speed: f32 = 0.9 in 0.0 ..= 5.0;
        muzzle_flash_width: f32 = 0.032 in 0.0 ..= 0.5;
        muzzle_flash_strength: f32 = 0.015 in 0.0 ..= 0.5;
        /// Half-extent (px) of the quad the muzzle flash is drawn into -
        /// must contain the ring's full reach (speed * duration, in screen
        /// px) plus its band width or it visibly clips.
        muzzle_flash_quad_radius: f32 = 90.0 in 10.0 ..= 500.0;
        /// Shell-impact flash (impact.fs): a one-sided punch plus a warm
        /// spark at the hit point - a sharp "thwack".
        impact_flash_duration: f32 = 0.14 in 0.01 ..= 2.0;
        impact_flash_speed: f32 = 1.1 in 0.0 ..= 5.0;
        impact_flash_width: f32 = 0.02 in 0.0 ..= 0.5;
        impact_flash_strength: f32 = 0.025 in 0.0 ..= 0.5;
        /// Half-extent (px) of the impact flash's quad; at 720px tall the
        /// punch reaches ~125px, so 70 visibly clipped it.
        impact_flash_quad_radius: f32 = 130.0 in 10.0 ..= 500.0;
    }
}

/// Values derived from other knobs - kept as methods (not their own rows)
/// so dragging the base knob moves them with it, exactly as the old
/// `const A = B * 0.8` definitions did.
impl Tuning {
    /// Engagement-slot ring radius: `enemy_attack_range * engage_ring_factor`.
    pub fn engage_ring_radius(&self) -> f32 {
        self.enemy_attack_range * self.engage_ring_factor
    }

    /// Reserve-rank distance: `enemy_attack_range + engage_reserve_extra_px`.
    pub fn engage_reserve_radius(&self) -> f32 {
        self.enemy_attack_range + self.engage_reserve_extra_px
    }

    /// Low-ammo retreat distance: `enemy_attack_range * enemy_retreat_range_factor`.
    pub fn enemy_retreat_range(&self) -> f32 {
        self.enemy_attack_range * self.enemy_retreat_range_factor
    }

    /// Fire cooldown held for a whole minigun burst: every queued bullet's
    /// delay plus the trailing gap.
    pub fn minigun_burst_cooldown_seconds(&self) -> f32 {
        (self.minigun_burst_size.saturating_sub(1)) as f32 * self.minigun_bullet_delay_seconds
            + self.minigun_burst_trailing_gap_seconds
    }

    /// Schema row for `name`, if it's a table row.
    pub fn meta(name: &str) -> Option<&'static ParamMeta> {
        Self::SCHEMA.iter().find(|m| m.name == name)
    }

    /// Apply a JSON patch object: any subset of keys, each a number (scalar
    /// row or `name.elem` element), a bool, or - for an array row - a
    /// full-length array. Validated key by key; the caller is expected to
    /// apply this to a *copy* and only commit on `Ok`, which is what
    /// [`submit_json`] does. Returns how many keys were applied.
    pub fn apply_patch(&mut self, patch: &Map<String, Value>) -> Result<usize, String> {
        for (key, value) in patch {
            match value {
                Value::Number(n) => {
                    let v = n.as_f64().ok_or_else(|| format!("{key}: not a finite number"))?;
                    self.set(key, v)?;
                }
                Value::Bool(b) => self.set(key, if *b { 1.0 } else { 0.0 })?,
                Value::Array(items) => {
                    let meta = Self::meta(key).ok_or_else(|| format!("unknown tunable {key:?}"))?;
                    if !meta.is_array() {
                        return Err(format!("{key} is a scalar, not an array"));
                    }
                    if items.len() != meta.labels.len() {
                        return Err(format!(
                            "{key}: expected {} elements, got {}",
                            meta.labels.len(),
                            items.len()
                        ));
                    }
                    for (i, item) in items.iter().enumerate() {
                        let v = item
                            .as_f64()
                            .or_else(|| item.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
                            .ok_or_else(|| format!("{key}[{i}]: not a number"))?;
                        self.set(&format!("{key}.{i}"), v)?;
                    }
                }
                _ => return Err(format!("{key}: expected a number, bool, or array")),
            }
        }
        Ok(patch.len())
    }

    /// The whole table as a flat JSON object (`serde_json::Value`).
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("Tuning serializes infallibly")
    }

    /// Only the rows that differ from [`Tuning::DEFAULT`], as a JSON object
    /// - what gets saved/shared. Empty object when everything is stock.
    pub fn diff_value(&self) -> Map<String, Value> {
        let current = self.to_value();
        let default = Self::DEFAULT.to_value();
        let (Value::Object(current), Value::Object(default)) = (current, default) else {
            unreachable!("Tuning serializes as an object");
        };
        current
            .into_iter()
            .filter(|(k, v)| default.get(k) != Some(v))
            .collect()
    }

    /// The differing rows rendered as `tunables!` table rows, so a value
    /// found in the dev panel pastes straight back into this file.
    pub fn diff_rust(&self) -> String {
        let diff = self.diff_value();
        let mut out = String::new();
        for meta in Self::SCHEMA {
            let Some(value) = diff.get(meta.name) else { continue };
            let default = match value {
                Value::Array(items) => {
                    let parts: Vec<String> = items.iter().map(|v| rust_literal(v, meta.kind)).collect();
                    format!("[{}]", parts.join(", "))
                }
                other => rust_literal(other, meta.kind),
            };
            let labels = if meta.is_array() {
                // The label set is the only `[&str; N]` whose length matches
                // - good enough for a paste-back snippet.
                match meta.labels.len() {
                    12 => " labels TANK_NAMES".to_string(),
                    4 => " labels MATERIAL_NAMES".to_string(),
                    n => format!(" labels /* {n} labels */"),
                }
            } else {
                String::new()
            };
            let applies = match meta.applies {
                Applies::Live => "",
                Applies::Spawn => " @ Spawn",
                Applies::Restart => " @ Restart",
            };
            out.push_str(&format!(
                "{}: {} = {} in {} ..= {}{}{};\n",
                meta.name,
                meta.ty,
                default,
                number_literal(meta.min, meta.kind),
                number_literal(meta.max, meta.kind),
                labels,
                applies
            ));
        }
        out
    }
}

/// A JSON number as a Rust literal of the row's kind (`99.0` for floats,
/// `23` for integers, `true`/`false` for bools).
fn rust_literal(v: &Value, kind: Kind) -> String {
    match kind {
        Kind::Bool => v.as_bool().or_else(|| v.as_f64().map(|f| f != 0.0)).unwrap_or(false).to_string(),
        _ => number_literal(v.as_f64().unwrap_or(0.0), kind),
    }
}

fn number_literal(v: f64, kind: Kind) -> String {
    match kind {
        Kind::F32 | Kind::F64 => {
            let s = format!("{v}");
            if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
        }
        Kind::Bool => (v != 0.0).to_string(),
        _ => format!("{}", v as i64),
    }
}

// ---------------------------------------------------------------------------
// Global store: one live table, one staged replacement, one restart flag.
// ---------------------------------------------------------------------------

static TUNING: RwLock<Tuning> = RwLock::new(Tuning::DEFAULT);
/// The next table, built up by `submit_*` calls since the last frame
/// boundary; `apply_pending` swaps it in. Staging on a copy means a batch of
/// submits between two frames all land together, and a rejected patch never
/// half-applies.
static STAGED: Mutex<Option<Tuning>> = Mutex::new(None);
static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The live table. Cheap (an uncontended read lock); bind once per function
/// in hot code. Never hold the guard across [`apply_pending`]/[`replace_now`]
/// (they take the write lock) - the main loop calls those between frames,
/// outside any read.
#[inline]
pub fn tuning() -> RwLockReadGuard<'static, Tuning> {
    TUNING.read().unwrap_or_else(PoisonError::into_inner)
}

/// A copy of the live table.
pub fn current() -> Tuning {
    *tuning()
}

/// Stage a JSON patch (see [`Tuning::apply_patch`]) on top of whatever is
/// already staged (or the live table), to land at the next
/// [`apply_pending`]. Rejected as a whole on any bad key/value, with the
/// key named in the error. Returns how many keys were applied.
pub fn submit_json(json: &str) -> Result<usize, String> {
    let patch: Value = serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;
    let Value::Object(patch) = patch else {
        return Err("expected a JSON object of {\"knob\": value} pairs".to_string());
    };
    let mut staged = STAGED.lock().unwrap_or_else(PoisonError::into_inner);
    let mut next = staged.unwrap_or_else(current);
    let applied = next.apply_patch(&patch)?;
    *staged = Some(next);
    Ok(applied)
}

/// Stage a full reset to [`Tuning::DEFAULT`].
pub fn submit_reset() {
    *STAGED.lock().unwrap_or_else(PoisonError::into_inner) = Some(Tuning::DEFAULT);
}

/// Ask the main loop to restart the round at the next frame boundary (see
/// [`take_restart_request`]) - the dev panel's "Restart round" button, so a
/// tuned set can be watched from a fresh spawn.
pub fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::Relaxed);
}

/// Swap any staged table in as the live one. Call once per frame, before
/// `Game::update`. Returns whether anything changed.
pub fn apply_pending() -> bool {
    let staged = STAGED.lock().unwrap_or_else(PoisonError::into_inner).take();
    match staged {
        Some(next) => {
            *TUNING.write().unwrap_or_else(PoisonError::into_inner) = next;
            true
        }
        None => false,
    }
}

/// Consume a pending restart request (true at most once per request).
pub fn take_restart_request() -> bool {
    RESTART_REQUESTED.swap(false, Ordering::Relaxed)
}

/// Replace the live table immediately, bypassing staging - for startup
/// (`--tuning <file>`) and the probe, before any frame has read it.
pub fn replace_now(t: Tuning) {
    *TUNING.write().unwrap_or_else(PoisonError::into_inner) = t;
}

/// Read a JSON file holding a patch object (typically a saved
/// [`diff_json`]) and stage it. Returns how many keys it set.
pub fn submit_file(path: &std::path::Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    submit_json(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// [`Tuning::SCHEMA`] as JSON: an array of rows, each the `ParamMeta`
/// fields plus that row's `default` value.
pub fn schema_json() -> String {
    let defaults = Tuning::DEFAULT.to_value();
    let rows: Vec<Value> = Tuning::SCHEMA
        .iter()
        .map(|m| {
            let mut row = serde_json::to_value(m).expect("ParamMeta serializes infallibly");
            if let Value::Object(obj) = &mut row {
                obj.insert("default".into(), defaults.get(m.name).cloned().unwrap_or(Value::Null));
            }
            row
        })
        .collect();
    Value::Array(rows).to_string()
}

/// The live table as a flat JSON object.
pub fn current_json() -> String {
    tuning().to_value().to_string()
}

/// Only the live rows that differ from the defaults, as a JSON object.
pub fn diff_json() -> String {
    Value::Object(tuning().diff_value()).to_string()
}

/// The differing rows as `tunables!` table rows (see [`Tuning::diff_rust`]).
pub fn diff_rust() -> String {
    tuning().diff_rust()
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests work on local `Tuning` values only - never on the global
    // store, since `cargo test` runs tests in parallel threads and the rest
    // of the suite (determinism, fixtures) reads the global at DEFAULT.

    #[test]
    fn schema_covers_every_field_exactly_once() {
        let value = Tuning::DEFAULT.to_value();
        let Value::Object(fields) = value else { panic!("not an object") };
        assert_eq!(fields.len(), Tuning::SCHEMA.len());
        let mut names: Vec<&str> = Tuning::SCHEMA.iter().map(|m| m.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Tuning::SCHEMA.len(), "duplicate row names");
        for m in Tuning::SCHEMA {
            assert!(fields.contains_key(m.name), "{} missing from serialization", m.name);
            assert!(m.min <= m.max, "{}: min > max", m.name);
        }
    }

    #[test]
    fn defaults_are_inside_their_ranges() {
        let t = Tuning::DEFAULT;
        for m in Tuning::SCHEMA {
            if m.is_array() {
                for (i, label) in m.labels.iter().enumerate() {
                    let v = t.get(&format!("{}.{label}", m.name)).unwrap();
                    assert!((m.min..=m.max).contains(&v), "{}[{i}]={v} outside range", m.name);
                }
            } else {
                let v = t.get(m.name).unwrap();
                assert!((m.min..=m.max).contains(&v), "{}={v} outside range", m.name);
            }
        }
    }

    #[test]
    fn scalar_set_get_and_range_check() {
        let mut t = Tuning::DEFAULT;
        t.set("tank_speed", 99.0).unwrap();
        assert_eq!(t.tank_speed, 99.0);
        assert_eq!(t.get("tank_speed"), Some(99.0));
        assert!(t.set("tank_speed", 1e9).unwrap_err().contains("outside"));
        assert!(t.set("max_shells", 23.0).is_ok());
        assert_eq!(t.max_shells, 23);
        assert!(t.set("nope", 1.0).unwrap_err().contains("unknown"));
        assert!(t.set("tank_speed.x", 1.0).unwrap_err().contains("scalar"));
        assert!(t.set("tank_speed", f64::NAN).is_err());
    }

    #[test]
    fn array_rows_address_by_label_or_index() {
        let mut t = Tuning::DEFAULT;
        t.set("tank_mass_factor.titan", 2.5).unwrap();
        assert_eq!(t.tank_mass_factor[10], 2.5);
        t.set("wall_max_health.2", 40.0).unwrap();
        assert_eq!(t.wall_max_health[2], 40.0);
        assert_eq!(t.get("wall_max_health.wood"), Some(40.0));
        assert!(t.set("wall_max_health.paper", 1.0).unwrap_err().contains("no element"));
        assert!(t.set("wall_max_health", 1.0).unwrap_err().contains("array"));
        assert_eq!(t.get("wall_max_health"), None);
    }

    #[test]
    fn json_patch_round_trips_through_diff() {
        let mut t = Tuning::DEFAULT;
        let patch: Value = serde_json::from_str(
            r#"{"tank_speed": 99, "max_shells": 23, "wall_max_health.wood": 40,
                "tank_damage_factor": [1,1,1,1,1,1,1,1,1,1,1,1]}"#,
        )
        .unwrap();
        let n = t.apply_patch(patch.as_object().unwrap()).unwrap();
        assert_eq!(n, 4);
        let diff = t.diff_value();
        assert_eq!(diff.len(), 4, "{diff:?}");
        assert_eq!(diff["tank_speed"], Value::from(99.0));
        assert_eq!(diff["wall_max_health"][2], Value::from(40.0));

        // Feeding the diff back into a fresh table reproduces it.
        let mut again = Tuning::DEFAULT;
        again.apply_patch(&diff).unwrap();
        assert_eq!(again, t);

        // A bad key rejects, and a partial-length array rejects.
        let bad: Value = serde_json::from_str(r#"{"tank_speed": 99, "bogus": 1}"#).unwrap();
        assert!(Tuning::default().apply_patch(bad.as_object().unwrap()).is_err());
        let short: Value = serde_json::from_str(r#"{"wall_max_health": [1, 2]}"#).unwrap();
        assert!(Tuning::default().apply_patch(short.as_object().unwrap()).is_err());
    }

    #[test]
    fn diff_rust_renders_pasteable_rows() {
        let mut t = Tuning::DEFAULT;
        t.set("tank_speed", 99.0).unwrap();
        t.set("max_shells", 23.0).unwrap();
        t.set("wall_max_health.wood", 40.0).unwrap();
        let rust = t.diff_rust();
        assert!(rust.contains("tank_speed: f32 = 99.0 in 20.0 ..= 800.0;"), "{rust}");
        assert!(rust.contains("max_shells: i32 = 23 in 1 ..= 100 @ Spawn;"), "{rust}");
        assert!(
            rust.contains("wall_max_health: [f32; 4] = [20.0, 220.0, 40.0, 2.0] in 1.0 ..= 1000.0 labels MATERIAL_NAMES @ Spawn;"),
            "{rust}"
        );
    }

    #[test]
    fn derived_values_track_their_base_knob() {
        let mut t = Tuning::DEFAULT;
        assert_eq!(t.engage_ring_radius(), 340.0 * 0.8);
        assert_eq!(t.engage_reserve_radius(), 400.0);
        assert_eq!(t.enemy_retreat_range(), 340.0 * 1.3);
        t.enemy_attack_range = 500.0;
        assert_eq!(t.engage_ring_radius(), 400.0);
        assert_eq!(t.minigun_burst_cooldown_seconds(), 5.0 * 0.04 + 0.1);
    }

    #[test]
    fn schema_json_is_an_array_of_rows_with_defaults() {
        let v: Value = serde_json::from_str(&schema_json()).unwrap();
        let rows = v.as_array().unwrap();
        assert_eq!(rows.len(), Tuning::SCHEMA.len());
        let speed = rows.iter().find(|r| r["name"] == "tank_speed").unwrap();
        assert_eq!(speed["group"], "movement");
        assert_eq!(speed["kind"], "f32");
        assert_eq!(speed["default"], Value::from(210.0));
        assert_eq!(speed["applies"], "live");
        assert!(speed["doc"].as_str().unwrap().contains("Player top speed"));
        let walls = rows.iter().find(|r| r["name"] == "wall_max_health").unwrap();
        assert_eq!(walls["labels"], serde_json::json!(["brick", "iron", "wood", "glass"]));
    }
}
