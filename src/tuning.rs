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
use std::sync::{Arc, LazyLock, Mutex, PoisonError, RwLock};

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
        enemy_count_min: usize = 4 in 0 ..= 30 @ Restart;
        enemy_count_max: usize = 7 in 0 ..= 30 @ Restart;
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
        /// Odds any tank (the player included) starts the round already
        /// under a rainbow shield (`Tank::shield_hp` set to
        /// `shield_capacity`). Rolled independently per tank.
        spawn_shield_chance: f32 = 0.05 in 0.0 ..= 1.0 @ Restart;
        /// Of an enemy that rolls a special weapon, the odds it's a laser.
        enemy_special_weapon_laser_share: f32 = 0.5 in 0.0 ..= 1.0 @ Restart;
        /// Of the remaining (non-laser) share, the odds it's plasma rather
        /// than a minigun. 0.5 means the two split the non-laser half evenly
        /// (laser 50%, plasma 25%, minigun 25% overall).
        enemy_special_weapon_plasma_share: f32 = 0.5 in 0.0 ..= 1.0 @ Restart;
        /// Which chassis the player spawns in, as a row index into
        /// `scifi_tanks_sheet.png`: 0 scout, 1 assault, 2 breaker,
        /// 3 longbow, 4 flak, 5 wraith, 6 warden, 7 ravager, 8 glacier,
        /// 9 obelisk, 10 titan, 11 leviathan (`TANK_NAMES` order). -1, the
        /// default, means "unset" - the chassis then comes from the loaded
        /// map's own `tank` key, or a random roll if the map sets none.
        /// `--tank` on the command line outranks this knob; nothing else
        /// does, so dragging this is how a browser round picks a chassis.
        player_tank: i32 = (-1) in -1 ..= 11 @ Restart;
        /// A drag shorter than this (UI points, `hud::UiFrame`) on both
        /// axes from the stick's origin is a resting thumb, not a
        /// direction.
        touch_dead_zone_pt: f32 = 14.0 in 4.0 ..= 40.0;
        /// The stick's origin trails the thumb so the drag never exceeds
        /// this many UI points: a change of direction costs the same short
        /// slide however far the thumb has pushed, and the drawn base sits
        /// where the rule measures from. 0 pins the origin where the thumb
        /// landed, and a long push then needs a long slide back.
        touch_follow_radius_pt: f32 = 40.0 in 0.0 ..= 120.0;
        /// Degrees off the held axis a drag has to reach before the other
        /// axis takes over - the hysteresis that keeps a drag near a
        /// diagonal from flickering. 45 is no band at all; past 60 a thumb
        /// reads the band as the tank refusing to turn.
        touch_axis_switch_deg: f32 = 50.0 in 45.0 ..= 75.0;
        /// When the round ends (player destroyed, or all enemies destroyed)
        /// the result counts down this long once its finale and its fade
        /// have played (`round_finale_seconds`,
        /// `round_verdict_fade_seconds`), then the game restarts - or, on a
        /// level, goes on to the next level after a win
        /// (`level_loss_retry_seconds` counts a lost level down).
        restart_delay: f32 = 3.0 in 0.0 ..= 30.0;
        /// A lost level's end screen counts this long before it plays the
        /// level again; a press takes it sooner.
        level_loss_retry_seconds: f32 = 5.0 in 0.0 ..= 30.0;
        /// Once the round is decided the world plays on this long with
        /// nothing over it, so the blast that decided it is seen whole.
        /// Nothing deals damage; a press or a tap skips to the verdict.
        round_finale_seconds: f32 = 2.4 in 0.0 ..= 6.0;
        /// After the finale the end screen's dim, title and numbers ease
        /// in over this long, and the corner clusters come back with them.
        round_verdict_fade_seconds: f32 = 0.5 in 0.0 ..= 2.0;
        /// The picture holds this long, in real seconds, on the blow that
        /// decided the round. 0 is no hold; reduced motion has none.
        round_hitstop_seconds: f32 = 0.05 in 0.0 ..= 0.5;
        /// Then the round runs at this share of its speed for
        /// `round_slowmo_seconds`, easing back to full speed. 1 is no slow
        /// motion; reduced motion has none.
        round_slowmo_scale: f32 = 0.5 in 0.1 ..= 1.0;
        round_slowmo_seconds: f32 = 0.5 in 0.0 ..= 2.0;
        /// The fade through black to another level, out and in, each this
        /// many seconds.
        level_fade_seconds: f32 = 0.4 in 0.0 ..= 2.0;
        /// The fade through black playing the same map again, out and in.
        retry_fade_seconds: f32 = 0.25 in 0.0 ..= 2.0;
    }

    group mission {
        /// How long the round stays frozen behind the opening mission
        /// banner ("PROTECT THE FROG!") before play starts. Any move or
        /// fire input skips it. Headless callers start with the intro off.
        mission_banner_seconds: f32 = 2.0 in 0.0 ..= 10.0;
        /// Protect mission: odds each enemy is rolled a hunter that drives
        /// at and shoots the frog rather than the player.
        enemy_hunter_share_protect: f32 = 0.25 in 0.0 ..= 1.0 @ Spawn;
        /// Hunt mission: odds each enemy is a hunter; the rest guard the
        /// enemy frog.
        enemy_hunter_share_hunt: f32 = 0.6 in 0.0 ..= 1.0 @ Spawn;
        /// A guard engages the player only while the player is within this
        /// many px of the enemy frog, and wanders inside it otherwise.
        guard_leash_px: f32 = 260.0 in 50.0 ..= 1000.0;
        /// A guard's beat keeps at least this far from its own frog -
        /// outside the frog's hop range, so it doesn't shove the thing it
        /// is guarding around the map. (Its own frog never bites it.)
        guard_keep_off_px: f32 = 130.0 in 0.0 ..= 500.0;
        /// Procedural enemy-frog placement (a hunt map without an
        /// `enemy_frog` cell): at least this far from the player's frog.
        enemy_frog_spawn_min_dist: f32 = 270.0 in 0.0 ..= 1500.0 @ Restart;
        /// After a hunter's opportunistic shot at the player it goes back
        /// to the frog for at least this long before it may snipe again,
        /// so a player parked on its firing axis can't hold it forever.
        hunter_snipe_cooldown_seconds: f32 = 5.0 in 0.0 ..= 60.0;
    }

    group waves {
        /// Defaults for a waves spawn plan whose map/CLI leave them unset:
        /// number of waves, first-wave size, tanks added per wave.
        wave_count_default: usize = 5 in 1 ..= 50 @ Restart;
        wave_size_default: usize = 3 in 1 ..= 31 @ Restart;
        wave_growth_default: usize = 1 in 0 ..= 10 @ Restart;
        /// Breather between a wave being cleared (or timing out) and the
        /// next one rolling in; the "WAVE N" banner shows meanwhile.
        wave_gap_seconds: f32 = 4.0 in 0.0 ..= 60.0;
        /// A wave that is not cleared within this long is joined by the
        /// next one anyway.
        wave_timeout_seconds: f32 = 60.0 in 1.0 ..= 600.0;
        /// The next wave is called once this many enemies (or fewer) are
        /// still alive.
        wave_next_when_alive: usize = 0 in 0 ..= 30;
        /// Seconds between two tanks of the same wave starting their
        /// roll-in, so they never overlap in one gate lane.
        wave_stagger_seconds: f32 = 0.8 in 0.0 ..= 10.0;
        /// Roll-in speed as a factor of the tank's normal driving speed.
        wave_rollin_speed_factor: f32 = 0.8 in 0.1 ..= 3.0;
        /// An edge nav cell is a gate only if this many cells inward are
        /// all open.
        wave_gate_inward_cells: usize = 3 in 1 ..= 10;
        /// Gates closer than this (px) to the player or the player's frog
        /// are skipped.
        wave_gate_min_player_dist: f32 = 300.0 in 0.0 ..= 1500.0;
        /// Odds a wave tank is drawn one tier below the wave's tier, for
        /// variety.
        wave_tier_mix: f32 = 0.25 in 0.0 ..= 1.0;
        /// Wave rounds only: a wreck fades out and is removed this long
        /// after the kill, keeping the field passable. 0 keeps wrecks.
        wave_wreck_despawn_seconds: f32 = 20.0 in 0.0 ..= 300.0;
        /// Cap on live enemies at once; a wave that would exceed it queues
        /// its surplus until slots free up.
        wave_max_alive: usize = 31 in 1 ..= 31;
        /// Every wave of the plan the map or the CLI authored is multiplied
        /// by this and rounded (the first wave's size and the tanks each
        /// wave adds, both at least what the plan asked for once rounded);
        /// 1.0 plays the plan as written.
        ///
        /// This is how a round is sized to the team that fights it
        /// (docs/online-coop-prd.md section 4.11): a room works it out from
        /// `online_wave_size_per_seat` and the seats its round starts with,
        /// and sends it in the `Welcome`'s tuning patch, so a local round -
        /// on a couch or in the probe - always runs at 1.0. Read once, when
        /// the plan is resolved in `Game::init`.
        wave_size_scale: f32 = 1.0 in 0.1 ..= 8.0 @ Restart;
        /// Rungs added to both ends of the wave plan's tier ramp, clamped
        /// to the top of the ladder: 1 starts a light -> super plan at
        /// medium. The room's other seat-count dial
        /// (`online_wave_tier_seats_per_step`), 0 on every local round.
        wave_tier_step: usize = 0 in 0 ..= 3 @ Restart;
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
        shell_speed: f32 = 522.5 in 50.0 ..= 3000.0;
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
        max_shells: i32 = 20 in 1 ..= 100 @ Spawn;
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
        /// A player's hull armour: every hit that lands on a seat's hull -
        /// shells, bullets, blasts, rams, fire, towers, frogs - is scaled
        /// by this (`Tank::take_damage`), so 0.77 makes a player's tank
        /// 30 % tougher (1 / 0.77). The shield is spent in full.
        player_armor_factor: f32 = 0.77 in 0.1 ..= 1.0;
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
        /// Rounds granted per minigun pickup: ten full bursts at
        /// `minigun_burst_size` 6, so each of the ring's ten ammo pips
        /// (`tank::AMMO_PIPS`) is one burst.
        minigun_ammo_per_pickup: i32 = 60 in 1 ..= 1000;
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
        minigun_bullet_speed: f32 = 595.65 in 50.0 ..= 5000.0;
        /// Where the minigun's line crosses the gun line, in px ahead of the
        /// pivot (`Bullet::spawn`): the module sits beside the main gun, so
        /// its bullets are boresighted onto the gun line here - nearer, a
        /// burst lands a little to the module's side of it, further, a
        /// little to the other. Around the enemies' engagement ring.
        minigun_boresight_px: f32 = 256.0 in 64.0 ..= 1024.0;
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
        /// How long each of the minigun module's 3 "hot barrel" cells is
        /// shown before advancing, while a burst is active (a discrete cell
        /// swap, not a rotation - see `tank::draw_tank`). Tuned close to
        /// `minigun_bullet_delay_seconds` so roughly one barrel swap
        /// happens per bullet.
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
        plasma_speed: f32 = 526.68 in 50.0 ..= 3000.0;
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
        /// The in-flight orb's breathing (`render/shot_shaders.rs`): pulses
        /// per second, and the strength of its glow and rim at the low and
        /// high point of each pulse.
        plasma_pulse_hz: f32 = 6.0 in 0.1 ..= 30.0;
        plasma_pulse_min_scale: f32 = 0.85 in 0.1 ..= 3.0;
        plasma_pulse_max_scale: f32 = 1.35 in 0.1 ..= 3.0;
        /// The Flying state's baked 4-frame breathing cycle plays this many
        /// full cycles per second, where the baked sprite is what flies (no
        /// shaders, or `shot_glow_strength` 0).
        plasma_flying_cycle_fps: f32 = 10.0 in 0.5 ..= 60.0;
    }

    group missiles {
        /// Seeker missiles granted per pickup - one volley from the
        /// four-tube pod.
        missile_ammo_per_pickup: i32 = 4 in 1 ..= 400;
        /// Missiles per salvo, one per tube, so at most the pod's four.
        /// The first leaves at once, the rest
        /// `missile_launch_delay_seconds` apart (`Tank::missile_volley`).
        missile_volley_size: u32 = 4 in 1 ..= 4;
        /// Salvos per trigger pull: the pod empties, reloads its tubes in
        /// `missile_salvo_gap_seconds` and fires again, so a pull is
        /// `missile_volley_size` x this many missiles.
        missile_salvos: u32 = 1 in 1 ..= 4;
        /// Gap between two missiles of one salvo leaving their tubes.
        missile_launch_delay_seconds: f32 = 0.05 in 0.0 ..= 1.0;
        /// Gap between one salvo's last missile and the next salvo's first.
        missile_salvo_gap_seconds: f32 = 0.18 in 0.0 ..= 2.0;
        /// Reload after a volley's last missile, before the next pull.
        missile_reload_seconds: f32 = 1.3 in 0.0 ..= 10.0;
        /// Degrees between neighbouring tubes' launch headings: the volley
        /// fans out as it climbs.
        missile_fan_deg: f32 = 12.0 in 0.0 ..= 45.0;
        /// How far apart (px) the missiles of one salvo come down: each
        /// tube aims this much beside its neighbour, across the line the
        /// missile locked along, so a salvo lands as a spread of blasts
        /// around the target rather than four on one spot.
        missile_impact_spread_px: f32 = 14.0 in 0.0 ..= 128.0;
        /// Stage one, the climb: how long a missile rises, how high it
        /// gets (px, drawn as lift above its ground point) and how fast it
        /// drifts along the launch heading meanwhile (px/s).
        missile_climb_seconds: f32 = 0.28 in 0.05 ..= 3.0;
        missile_apex_height: f32 = 72.0 in 0.0 ..= 300.0;
        missile_climb_speed: f32 = 156.75 in 0.0 ..= 1000.0;
        /// Stage two, the seek: seconds a missile hangs at the apex
        /// looking for a target, and how far from itself it looks (px).
        /// It locks the nearest opposing tank in range; with none, it
        /// keeps the ground point `missile_fallback_range` px ahead of the
        /// launcher and comes down there.
        missile_acquire_seconds: f32 = 0.08 in 0.0 ..= 2.0;
        missile_seek_range: f32 = 676.0 in 16.0 ..= 3000.0;
        missile_fallback_range: f32 = 338.0 in 16.0 ..= 3000.0;
        /// Stage three, the chase: top ground speed (px/s), how fast it
        /// gets there (px/s^2) and how fast it turns (degrees/s), the turn
        /// rate growing by `missile_turn_rate_growth_deg` every second of
        /// the chase so a missile circling its target always tightens in.
        missile_speed: f32 = 344.85 in 20.0 ..= 3000.0;
        missile_accel: f32 = 1100.0 in 1.0 ..= 10000.0;
        missile_turn_rate_deg: f32 = 200.0 in 1.0 ..= 3600.0;
        missile_turn_rate_growth_deg: f32 = 240.0 in 0.0 ..= 3600.0;
        /// The missile comes down over this last stretch to its target
        /// (px): full height beyond it, the ground at the end.
        missile_dive_distance: f32 = 150.0 in 1.0 ..= 1000.0;
        /// Inside this distance (px) it stops tracking and dives on the
        /// spot it last saw the target at - a tank that keeps moving can
        /// still slip the blast.
        missile_commit_distance: f32 = 56.0 in 0.0 ..= 500.0;
        /// A chase that has not come down after this long commits to a
        /// dive straight ahead.
        missile_max_flight_seconds: f32 = 5.2 in 0.5 ..= 30.0;
        /// Each missile's blast: radius (px), centre damage (falling off
        /// linearly to 0 at the edge) and the shove. Only the side opposing
        /// the shooter is hurt; everything in range is shoved, and tiles
        /// crack like under any blast.
        missile_blast_radius: f32 = 44.0 in 0.0 ..= 400.0;
        missile_blast_damage_min: f32 = 7.0 in 0.0 ..= 100.0;
        missile_blast_damage_max: f32 = 13.0 in 0.0 ..= 100.0;
        missile_blast_knockback_speed: f32 = 60.0 in 0.0 ..= 400.0;
        /// Size of a missile's fireball and scorch against a barrel's.
        missile_blast_fx_scale: f32 = 0.55 in 0.1 ..= 2.0;
        /// Launch kick per missile - small, a volley is four of them.
        missile_recoil_speed: f32 = 6.0 in 0.0 ..= 200.0;
        missile_recoil_max_speed: f32 = 14.0 in 0.0 ..= 400.0;
        /// How much bigger a missile draws at the top of its climb than on
        /// the ground - nearer the camera.
        missile_apex_draw_scale: f32 = 1.35 in 1.0 ..= 3.0;
        /// How quickly (1/s) a missile's sprite turns to point along the
        /// path it is drawn on - up out of the tube, level at the apex,
        /// down into the dive. Higher follows the path more tightly.
        missile_facing_smoothing: f32 = 18.0 in 1.0 ..= 120.0;
        /// The smoke trail each missile leaves in the air: one puff every
        /// this many px of flight (0 turns it off), each hanging where it
        /// was left for `missile_trail_seconds`, at this opacity.
        missile_trail_spacing: f32 = 4.0 in 0.0 ..= 64.0;
        missile_trail_seconds: f32 = 1.0 in 0.05 ..= 10.0;
        missile_trail_opacity: f32 = 0.75 in 0.0 ..= 1.0;
        missile_shadow_opacity: f32 = 0.3 in 0.0 ..= 1.0;
    }

    group grenades {
        /// Grenades one grenade crate loads into the launcher's drum
        /// (`pickup::PickupKind::Grenades`, `grenade.rs`). One per press.
        grenade_ammo_per_pickup: i32 = 6 in 1 ..= 40;
        /// Seconds between two launches.
        grenade_reload_seconds: f32 = 0.6 in 0.0 ..= 10.0;
        /// How fast a grenade leaves the barrel over the ground (px/s),
        /// plus this share of the launching hull's own velocity.
        grenade_launch_speed: f32 = 168.0 in 0.0 ..= 1500.0;
        grenade_launch_carry: f32 = 1.0 in 0.0 ..= 2.0;
        /// The lob: the height it leaves the barrel at (px), how fast it
        /// climbs (px/s) and the gravity that brings it down (px/s^2). In
        /// the air it flies over walls, props and tanks; the defaults carry
        /// it about 105 px, 36 px up at the top.
        grenade_launch_height: f32 = 6.0 in 0.0 ..= 100.0;
        grenade_launch_climb: f32 = 200.0 in 0.0 ..= 2000.0;
        grenade_gravity: f32 = 640.0 in 1.0 ..= 5000.0;
        /// Coming down it hops back up at this share of its fall while the
        /// fall is faster than `grenade_hop_min_speed` (px/s), and keeps
        /// `grenade_landing_keep` of its speed over the ground each landing.
        grenade_ground_bounce: f32 = 0.35 in 0.0 ..= 1.0;
        grenade_hop_min_speed: f32 = 60.0 in 0.0 ..= 1000.0;
        grenade_landing_keep: f32 = 0.75 in 0.0 ..= 1.0;
        /// How much bigger a grenade draws at `grenade_draw_lift_px` up and
        /// over - nearer the camera.
        grenade_apex_draw_scale: f32 = 1.3 in 1.0 ..= 3.0;
        grenade_draw_lift_px: f32 = 40.0 in 1.0 ..= 500.0;
        /// The ball's radius (px): what it bounces off walls and hulls by.
        grenade_radius: f32 = 12.0 in 1.0 ..= 32.0;
        /// How quickly a rolling grenade slows (1/s): its speed falls by
        /// this share a second, and under `grenade_stop_speed` px/s it
        /// comes to rest.
        grenade_roll_drag: f32 = 1.1 in 0.0 ..= 20.0;
        grenade_stop_speed: f32 = 6.0 in 0.0 ..= 100.0;
        /// The drag in water, and on ice, as multiples of the ground's:
        /// a grenade wallows in a ford and skates over a frozen lake.
        grenade_water_drag_factor: f32 = 4.0 in 0.0 ..= 50.0;
        grenade_ice_drag_factor: f32 = 0.25 in 0.0 ..= 5.0;
        /// The speed a grenade keeps off a wall, a tile or the field's
        /// edge (along the face it struck), and off a hull - which also
        /// hands it the hull's own motion, so a tank driving into one
        /// pushes it along and knocks it away.
        grenade_wall_restitution: f32 = 0.7 in 0.0 ..= 1.0;
        grenade_tank_restitution: f32 = 0.6 in 0.0 ..= 1.0;
        /// A grenade never rolls faster than this (px/s).
        grenade_max_speed: f32 = 600.0 in 1.0 ..= 3000.0;
        /// Seconds from launch to the blast.
        grenade_fuse_seconds: f32 = 6.0 in 0.1 ..= 30.0;
        /// The lamp's blink (Hz) at launch and at the blast: it quickens
        /// across the fuse, so how fast it flashes is how soon it goes.
        grenade_blink_hz_start: f32 = 1.5 in 0.1 ..= 30.0;
        grenade_blink_hz_end: f32 = 10.0 in 0.1 ..= 30.0;
        /// The blast: radius (px), centre damage (falling off linearly to
        /// 0 at the edge) and the shove. Only the side opposing the
        /// launcher is hurt; everything in range is shoved, and tiles crack
        /// like under any blast.
        grenade_blast_radius: f32 = 128.0 in 0.0 ..= 400.0;
        grenade_blast_damage_min: f32 = 30.0 in 0.0 ..= 100.0;
        grenade_blast_damage_max: f32 = 50.0 in 0.0 ..= 100.0;
        grenade_blast_knockback_speed: f32 = 240.0 in 0.0 ..= 500.0;
        /// Size of the fireball and scorch against a barrel's, and how
        /// hard its shockwave ripples and shakes against a tank dying.
        grenade_blast_fx_scale: f32 = 1.5 in 0.1 ..= 3.0;
        grenade_shock: f32 = 1.0 in 0.0 ..= 2.0;
        /// The white plume a grenade trails while in the air or rolling
        /// faster than `grenade_trail_min_speed` (px/s): a puff every this
        /// many px (0 turns it off), each swelling and gone in
        /// `grenade_trail_seconds` - thick, but much shorter than a
        /// missile's trail.
        grenade_trail_spacing: f32 = 5.0 in 0.0 ..= 64.0;
        grenade_trail_seconds: f32 = 0.55 in 0.05 ..= 10.0;
        grenade_trail_min_speed: f32 = 80.0 in 0.0 ..= 1000.0;
        /// Launch kick.
        grenade_recoil_speed: f32 = 10.0 in 0.0 ..= 200.0;
        grenade_recoil_max_speed: f32 = 20.0 in 0.0 ..= 400.0;
    }

    group flamethrower {
        /// Seconds of burn one flamethrower pickup grants; a second pickup
        /// stacks. The weapon is stocked while any fuel is left, and the
        /// HUD shows whole seconds.
        flame_fuel_per_pickup: f32 = 9.0 in 0.5 ..= 60.0;
        /// Length of the cone (px) from the muzzle; a solid tile on the
        /// centre line caps it for that frame.
        flame_range: f32 = 164.0 in 16.0 ..= 400.0;
        /// Half angle of the cone, degrees.
        flame_half_angle_deg: f32 = 21.4 in 2.0 ..= 45.0;
        /// Damage per second to a tank inside the cone (a fixed rate, no
        /// roll - the flamethrower draws no RNG).
        flame_damage_per_second: f32 = 20.0 in 0.0 ..= 200.0;
        /// How long a tank keeps burning after the stream touched it, and
        /// what that costs per second. Re-contact resets the timer.
        flame_afterburn_seconds: f32 = 2.75 in 0.0 ..= 20.0;
        flame_afterburn_dps: f32 = 4.0 in 0.0 ..= 100.0;
        /// Damage per second to a frog inside the cone.
        flame_frog_damage_per_second: f32 = 10.0 in 0.0 ..= 100.0;
        /// Seconds of exposure a ground cell or a tile needs before it
        /// catches: a quick sweep scorches, a held stream lights.
        flame_ignite_seconds: f32 = 0.35 in 0.0 ..= 5.0;
        /// How fast exposure fades (per second) once the stream moves off.
        flame_heat_decay: f32 = 1.0 in 0.0 ..= 10.0;
        /// How long a ground cell the stream lit burns (as a pool cell).
        flame_ground_seconds: f32 = 2.0 in 0.1 ..= 30.0;
        /// Exposure that collapses a sandbag, and that snaps a fence.
        flame_sandbag_seconds: f32 = 1.7 in 0.1 ..= 30.0;
        flame_fence_seconds: f32 = 1.2 in 0.1 ..= 30.0;
        /// Stream particles a second (cosmetic).
        flame_particle_rate: f32 = 436.0 in 0.0 ..= 900.0;
        /// A muzzle heat shimmer is pushed every this many frames while the
        /// trigger is held, so the ripple list is not flooded.
        flame_shimmer_every_frames: i32 = 11 in 1 ..= 60;
    }

    group towers {
        /// Tesla coil toughness (docs/defence-towers-prd.md). Baked in at
        /// spawn.
        tesla_max_health: f32 = 120.0 in 1.0 ..= 1000.0 @ Spawn;
        /// How close (px, centre to hull box) an opposing tank has to come
        /// before the coil starts charging at it.
        tesla_range: f32 = 112.0 in 32.0 ..= 600.0;
        /// Seconds from an empty coil to a strike with a target in reach:
        /// the telegraph a player backs out on.
        tesla_charge_seconds: f32 = 1.3 in 0.1 ..= 10.0;
        /// Charge lost per second while nothing is in reach.
        tesla_drain_per_second: f32 = 1.0 in 0.0 ..= 10.0;
        /// Pause after a strike before the coil charges again.
        tesla_cooldown_seconds: f32 = 0.35 in 0.0 ..= 10.0;
        /// Strike damage, rolled per strike.
        tesla_damage_min: f32 = 18.0 in 0.0 ..= 100.0;
        tesla_damage_max: f32 = 26.0 in 0.0 ..= 100.0;
        /// Extra tanks a strike jumps to after the first; 0 turns the
        /// chain off.
        tesla_chain_jumps: i32 = 1 in 0 ..= 4;
        /// How far a jump reaches from the tank it leaves (px).
        tesla_chain_radius: f32 = 64.0 in 0.0 ..= 300.0;
        /// Damage of each jump, as a fraction of the strike's roll.
        tesla_chain_factor: f32 = 0.5 in 0.0 ..= 1.0;
        /// How long a bolt stays on screen.
        tesla_bolt_display_seconds: f32 = 0.24 in 0.05 ..= 1.0;
        /// Odds a shell or bullet glances off a coil.
        tesla_deflect_chance: f64 = 0.15 in 0.0 ..= 1.0;
        /// The discharge when a coil dies: radius and damage at the centre
        /// (linear falloff), to every tank of either side.
        tesla_death_blast_radius: f32 = 64.0 in 0.0 ..= 300.0;
        tesla_death_blast_damage: f32 = 10.0 in 0.0 ..= 100.0;
        /// Gun tower toughness. Baked in at spawn.
        gun_tower_max_health: f32 = 150.0 in 1.0 ..= 1000.0 @ Spawn;
        /// How far the gun tower shoots (px, centre to centre).
        gun_tower_range: f32 = 256.0 in 32.0 ..= 1200.0;
        /// Turret turn rate, degrees a second.
        gun_tower_turn_deg_per_second: f32 = 220.0 in 10.0 ..= 2000.0;
        /// How far the gun aims ahead of a moving target: 0 aims where the
        /// tank is, 1 where it will be when the bullet gets there.
        gun_tower_lead: f32 = 0.5 in 0.0 ..= 1.0;
        /// Aim error (degrees) the gun opens fire within.
        gun_tower_fire_cone_deg: f32 = 6.0 in 0.0 ..= 45.0;
        /// Bullets per burst, the gap between them and the pause after.
        gun_tower_burst_size: i32 = 5 in 1 ..= 30;
        gun_tower_bullet_delay_seconds: f32 = 0.06 in 0.01 ..= 1.0;
        gun_tower_burst_gap_seconds: f32 = 0.9 in 0.0 ..= 10.0;
        /// Spread either side of the aim, degrees, rolled per bullet.
        gun_tower_spread_deg: f32 = 3.0 in 0.0 ..= 45.0;
        /// Bullet damage, rolled per hit.
        gun_tower_damage_min: f32 = 3.0 in 0.0 ..= 100.0;
        gun_tower_damage_max: f32 = 5.0 in 0.0 ..= 100.0;
        /// The gun holds fire while a tank of its own side is this close
        /// (px) to the line to its target.
        gun_tower_friendly_block_px: f32 = 20.0 in 0.0 ..= 100.0;
        /// Odds a shell or bullet glances off the gun tower's armour.
        gun_tower_deflect_chance: f64 = 0.25 in 0.0 ..= 1.0;
        /// Bio slush toughness. Baked in at spawn.
        bio_max_health: f32 = 130.0 in 1.0 ..= 1000.0 @ Spawn;
        /// Where the bio slush can land a glob: no closer than the first,
        /// no further than the second (px, centre to centre).
        bio_min_range: f32 = 56.0 in 0.0 ..= 600.0;
        bio_range: f32 = 192.0 in 32.0 ..= 1200.0;
        /// Nozzle turn rate, degrees a second.
        bio_turn_deg_per_second: f32 = 160.0 in 10.0 ..= 2000.0;
        /// Aim error (degrees) it lobs within.
        bio_fire_cone_deg: f32 = 12.0 in 0.0 ..= 45.0;
        /// Seconds between globs.
        bio_lob_interval_seconds: f32 = 2.2 in 0.2 ..= 20.0;
        /// A glob's time in the air and the drawn height at the top of its
        /// arc (px). It has no hit test in flight.
        bio_glob_flight_seconds: f32 = 0.8 in 0.1 ..= 5.0;
        bio_glob_apex_px: f32 = 28.0 in 0.0 ..= 200.0;
        /// How far the glob aims ahead of a moving target over its flight.
        bio_lead: f32 = 0.7 in 0.0 ..= 1.0;
        /// Hashed miss around the aim point (px); no RNG.
        bio_scatter_px: f32 = 10.0 in 0.0 ..= 64.0;
        /// Splash reach (px) for coating tanks and laying puddles.
        bio_splash_radius: f32 = 36.0 in 0.0 ..= 160.0;
        /// Damage of the splash itself (fixed, no roll).
        bio_splash_damage: f32 = 4.0 in 0.0 ..= 100.0;
        /// How long a coat of ooze lasts on a tank, what it corrodes a
        /// second, and the fraction of its pace the tank keeps. A new coat
        /// resets the timer, never adds to it.
        bio_slime_seconds: f32 = 4.0 in 0.0 ..= 30.0;
        bio_slime_dps: f32 = 3.0 in 0.0 ..= 50.0;
        bio_slime_speed_factor: f32 = 0.6 in 0.1 ..= 1.0;
        /// How long a puddle lasts, and what each step through one costs
        /// the router (it never blocks).
        bio_puddle_seconds: f32 = 6.0 in 0.0 ..= 60.0;
        bio_puddle_path_cost: usize = 3 in 0 ..= 64;
        /// How long the spill a destroyed bio slush leaves lasts.
        bio_spill_seconds: f32 = 12.0 in 0.0 ..= 120.0;
        /// Odds a shell or bullet glances off the bio slush.
        bio_deflect_chance: f64 = 0.1 in 0.0 ..= 1.0;
        /// Target hysteresis for the gun and the bio slush: a turret only
        /// switches to a new target this much (px) nearer than its current
        /// one.
        tower_switch_margin_px: f32 = 48.0 in 0.0 ..= 400.0;
        /// Health fraction below which a tower catches fire.
        tower_burn_below: f32 = 0.25 in 0.0 ..= 1.0;
        /// Damage a burning tower takes a second, until repaired or dead.
        tower_burn_dps: f32 = 2.0 in 0.0 ..= 50.0;
        /// Fraction of its rate a burning tower still fires at.
        tower_burning_fire_factor: f32 = 0.5 in 0.0 ..= 1.0;
        /// How long a ruin smoulders.
        tower_ruin_smoke_seconds: f32 = 8.0 in 0.0 ..= 60.0;
        /// Odds a tower pack drops beside a Health slot when it spawns,
        /// while a player tower is hurt. Gated before any RNG draw.
        tower_pack_near_health_chance: f32 = 0.25 in 0.0 ..= 1.0;
        /// Route surcharge on every cell a live player tower reaches, so
        /// enemies come round its reach when there is another way; 0 turns
        /// it off. Kept low on purpose: a gun tower's reach is a disc eight
        /// cells across, and a steep price on all of it sends every enemy
        /// down the same cheapest seam, where they pile up.
        route_tower_cost: usize = 2 in 0 ..= 64;
        /// Seconds an enemy a tower hit goes after that tower while no
        /// player is within its attack range.
        enemy_tower_grudge_seconds: f32 = 4.0 in 0.0 ..= 30.0;
    }

    group pickups {
        /// Seconds after a pickup is collected before a fresh one spawns at
        /// a random empty map slot - keeps the field topped up.
        pickup_respawn_seconds: f32 = 15.0 in 0.0 ..= 120.0;
        /// A pickup is collected once a tank's hull box, grown by this many
        /// px on every side, overlaps the pickup's 32 px square
        /// (`Pickup::in_reach`): touching it is enough, from the side, the
        /// front or a corner alike. Enemies take the same test.
        pickup_collect_pad_px: f32 = 6.0 in 0.0 ..= 64.0;
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
        /// Rainbow shield: collecting one heals the tank to full and *sets*
        /// `Tank::shield_hp` to this (a second one refills it rather than
        /// stacking). The shield is a pool of absorption on a clock
        /// (`shield_seconds`) - it soaks damage until spent or the clock
        /// runs out, whichever comes first. At the default an enemy's
        /// shell deflected costs 20 (`shield_deflect_cost_factor`), so
        /// three or four of them shatter it. See `Tank::take_damage` for
        /// the absorb path and `Game::resolve_projectiles` for the deflect
        /// one.
        shield_capacity: f32 = 70.0 in 0.0 ..= 1000.0;
        /// The longest a shield lasts, in seconds, however little it has
        /// been hit: it shatters when this runs out (`Tank::tick_shield`).
        shield_seconds: f32 = 6.0 in 0.5 ..= 60.0;
        /// What a *deflected* projectile costs the shield, as a multiple of
        /// the shot's own mid-range damage - shells, bullets and plasma
        /// bounce off (`Event::Deflected`) rather than landing, and pay this
        /// instead. Above 1 on purpose: shooting a shielded tank is then the
        /// fastest way to strip it *and* the most dangerous, since the shot
        /// comes back under the shielded tank's ownership. At 1 it would be
        /// strictly worse than any other damage source and nobody would
        /// ever shoot.
        shield_deflect_cost_factor: f32 = 2.0 in 0.0 ..= 10.0;
        /// How long a shield must go without absorbing anything before it
        /// starts refilling - the window that makes breaking contact worth
        /// something.
        shield_recharge_delay_seconds: f32 = 4.0 in 0.0 ..= 60.0;
        /// Refill rate once `shield_recharge_delay_seconds` has elapsed.
        /// Only a *live* shield recharges: once it shatters it is gone until
        /// another pickup, which is what stops this making shields
        /// permanent.
        shield_recharge_per_second: f32 = 20.0 in 0.0 ..= 200.0;
        /// Odds that a rainbow shield pickup is dropped in a free cell next
        /// to a Health slot each time that slot is spawned or respawned.
        /// The shield is an un-slotted bonus: it never respawns on its own,
        /// only ever alongside a health pack.
        shield_near_health_chance: f32 = 0.2 in 0.0 ..= 1.0;
        /// Frog health pack (docs/frog-health-pack-prd.md): the fraction of
        /// `frog_max_health` one pack restores to the collector's own frog.
        /// 1 is the agreed full heal; lower it to make the frog a running
        /// cost rather than something one detour resets.
        frog_pack_heal_fraction: f32 = 1.0 in 0.0 ..= 1.0;
        /// Odds that a frog health pack is dropped in a free cell next to a
        /// Health slot each time that slot is spawned or respawned - the
        /// same un-slotted bonus mechanism the rainbow shield uses, with
        /// one extra gate: it is only rolled while the frog is hurt (see
        /// `frog_pack_bonus_below`), so a round whose frog is never touched
        /// draws exactly the RNG it drew before this pickup existed. 0
        /// turns the bonus off and leaves only the map's own slots.
        frog_pack_near_health_chance: f32 = 0.35 in 0.0 ..= 1.0;
        /// How hurt the frog has to be before that bonus rolls at all, as a
        /// fraction of its max health.
        frog_pack_bonus_below: f32 = 0.75 in 0.0 ..= 1.0;
    }

    group crates {
        /// Seconds from a crate appearing to its landing: the air drop it
        /// comes down in (`crate_fx::drop` - a shadow gathers, the crate
        /// falls into it, squashes and throws a ring of dust). Only the
        /// drawing waits on it; the crate can be taken the frame it
        /// appears. 0 turns the drop off.
        crate_drop_seconds: f32 = 0.62 in 0.0 ..= 3.0;
        /// How high the crate falls from, px - drawn that far above its
        /// cell, and larger the higher it is.
        crate_drop_height_px: f32 = 110.0 in 0.0 ..= 400.0;
        /// Seconds from a crate being taken to its symbol landing in the
        /// tank that took it (`crate_fx::open`): the flash, the planks
        /// flying off, the symbol rising and blinking. 0 turns it off.
        crate_open_seconds: f32 = 0.75 in 0.0 ..= 3.0;
        /// A standing crate's glint sweeps its lid about this often,
        /// seconds (each crate on its own beat). 0 turns it off.
        crate_glint_period_seconds: f32 = 3.2 in 0.0 ..= 30.0;
        /// Seconds per frame of the glint's four.
        crate_glint_frame_seconds: f32 = 0.06 in 0.01 ..= 1.0;
        /// Breakable crates (docs/CRATES_SPEC.md): a blast or fire breaks a
        /// pickup's crate - shells and bullets still fly over it. Ordnance
        /// and energy cook off in a blast of their own; the rest spill,
        /// lying loose for `crate_spill_seconds`. Off, a crate is
        /// unbreakable and every round replays as it did. Read when a
        /// round begins.
        crate_breakable: bool = false in 0 ..= 1 @ Restart;
        /// A crate's hit points: a blast takes its mid damage times its
        /// falloff off them (no roll). At 8 an oil drum next to it breaks
        /// it, a missile landing on it does, a crate cooking off beside it
        /// does, and a dying tank's blast alone does not.
        crate_hp: f32 = 8.0 in 1.0 ..= 500.0;
        /// Seconds of the flamethrower's heat on a crate's cell before it
        /// catches; a burning ground cell under it lights it at once.
        crate_ignite_seconds: f32 = 0.5 in 0.0 ..= 10.0;
        /// Seconds a burning crate holds before it falls in.
        crate_burn_seconds: f32 = 2.0 in 0.1 ..= 20.0;
        /// Seconds a broken crate's contents lie loose, takeable, before
        /// they are gone and the slot refills as usual.
        crate_spill_seconds: f32 = 8.0 in 0.5 ..= 60.0;
        /// A crate cooking off: its blast as a fraction of an oil drum's -
        /// radius, damage and knockback.
        crate_cookoff_scale: f32 = 0.75 in 0.0 ..= 2.0;
    }

    group combat {
        /// Ramming: after taking collision damage a tank is immune for this
        /// long, so continuous touching doesn't drain damage every frame.
        ram_damage_cooldown: f32 = 0.5 in 0.0 ..= 5.0;
        /// Two-player rounds: what one player's shell, beam, bullet or ram
        /// deals to the other player, as a factor of the normal roll. 1 is
        /// full friendly fire, 0 makes teammates harmless to each other.
        friendly_fire_damage_factor: f32 = 1.0 in 0.0 ..= 2.0;
        /// Extra half-extent (px) on an enemy's hull and turret boxes for a
        /// player's shot only (`hits::Terrain::sweep`), so a shell whose
        /// sprite touches the hull lands instead of passing a pixel wide.
        /// Enemy shots, other seats' hulls, tiles and walls keep the exact
        /// boxes: this loosens the player's aim and nothing else.
        player_shot_hit_pad_px: f32 = 8.0 in 0.0 ..= 32.0;
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
        /// Linear and angular damping applied to a tank's body the frame it
        /// becomes a wreck. Nothing else in the world sets damping (rapier
        /// defaults to none) and a wreck stops being driven, so a shoved
        /// hulk used to slide until something stopped it. This is what
        /// makes a shove move it a little and then let it rest.
        wreck_linear_damping: f32 = 4.0 in 0.0 ..= 30.0;
        /// Surface friction on a wreck, against rapier's 0.5 default: a
        /// live tank shunting one should bleed energy rather than skid off
        /// it.
        wreck_friction: f32 = 0.95 in 0.0 ..= 2.0;
    }

    group ai {
        /// Start chasing the player within this distance (px). Was 520; at
        /// that value enemies never noticed the player past roughly half the
        /// window and read as passive.
        enemy_view_range: f32 = 800.0 in 50.0 ..= 3000.0;
        /// Two-player rounds: an enemy switches to the other player only
        /// once that player is this many px nearer than its current target
        /// - hysteresis, so two players at equal range do not flip the pack
        /// between them every frame.
        enemy_target_switch_margin_px: f32 = 96.0 in 0.0 ..= 400.0;
        /// Extra route cost on every cell a player's barrel points down
        /// (out to `route_lane_cells`, stopping at the first blocked cell),
        /// so the shared flow field brings enemies in from the flank
        /// instead of straight up the line of fire. A cell normally costs
        /// 1; 0 switches the lane layer off.
        route_lane_cost: usize = 3 in 0 ..= 64;
        /// How many cells ahead of a player's barrel the lane surcharge
        /// reaches.
        route_lane_cells: usize = 8 in 1 ..= 40;
        /// Extra route cost on the cell each live enemy stands in, so
        /// routes bend around a clump rather than queue through it. 0
        /// switches it off.
        route_crowd_cost: usize = 2 in 0 ..= 64;
        /// Stop and fight within this distance (px). The engagement ring and
        /// retreat range are factors of this - see the `engage` group.
        enemy_attack_range: f32 = 340.0 in 50.0 ..= 2000.0;
        /// Fire when the player is within this many px of the firing axis.
        enemy_fire_align_px: f32 = 24.0 in 1.0 ..= 200.0;
        /// The sight box around every seat, in cells from the seat's
        /// centre: this many sideways (`sight_box_half_cols`) and up and
        /// down (`sight_box_half_rows`), +-368 x +-240 px
        /// (`Tuning::sight_box_half_px`; docs/large-maps-follow-camera.md
        /// section 5). An enemy fires at a seat only while its centre stands
        /// inside that seat's box - a tank's attack and a hunter's snipe,
        /// a seeker missile choosing a seat to lock onto, an enemy tower
        /// choosing a seat to shoot - and every screen shows at least the
        /// box around its own seat, so nobody is shot from beyond the edge
        /// of their screen. Sideways it is wider than `enemy_attack_range`
        /// and changes nothing; up and down an enemy closes to 240 px
        /// before it fires, and the engagement ring's north and south
        /// firing slots stand inside it. A rule of the round, never of a
        /// window: the round that simulates the enemies applies it - the
        /// room, online - and every screen frames its own seat's box from
        /// the table it plays with, which is the build's unless a tester's
        /// panel or `--tuning` file changes it on that screen alone (a
        /// room's patch, `Welcome::tuning_json`, carries only the rows that
        /// size its waves).
        sight_box_half_cols: f32 = 11.5 in 1.0 ..= 64.0;
        sight_box_half_rows: f32 = 7.5 in 1.0 ..= 64.0;
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
        /// Field maps only: a hull reads its route as lanes, turning where
        /// its slide through the turn ends on the centre line of the cell
        /// the route turns in, wherever it rides across its lane - a flow
        /// field's route always, a searched one where the margin above
        /// never could turn it - and judges a wall ahead from where that
        /// slide leaves it (`ai::Ai::lane_turn`, `Ai::walks_into_wall`;
        /// docs/large-maps-follow-camera.md section 12). The margin decides
        /// the rest, and everything on an arena. Off, every switch is the
        /// margin's.
        ai_lane_turns: bool = true in 0 ..= 1;
        /// A committed heading about to walk into a known-blocked grid cell
        /// can be overridden, but only after this much dwell time - much
        /// shorter than `ai_dir_hold_seconds`, yet without some floor a
        /// coarse grid's routed direction flip-flops every frame near a
        /// corner (found via the probe's `--rounds` sweep).
        ai_obstacle_override_hold_seconds: f32 = 0.1 in 0.0 ..= 2.0;
        /// Stuck-escape: a tank commanded to move whose displacement per
        /// second along the commanded heading (smoothed, see below) stays
        /// under this - sideways drift in a jam is not progress ...
        stuck_speed_eps: f32 = 8.0 in 0.0 ..= 100.0;
        /// ... for this many seconds running is treated as genuinely stuck,
        /// and `Ai::steer` forces a hard perpendicular-turn reset.
        stuck_escape_seconds: f32 = 0.75 in 0.05 ..= 10.0;
        /// Time constant of the progress average the stuck check reads:
        /// long enough that a one-frame shove from the contact solver (two
        /// tanks pressed together twitch a pixel or two now and then)
        /// cannot clear the clock, short enough that a tank that really
        /// gets going clears it within a few frames.
        stuck_progress_window_seconds: f32 = 0.5 in 0.02 ..= 3.0;
        /// Breach: a tank that has commanded movement straight into a
        /// destructible tile for this long stops and shoots it down
        /// instead of staying wedged (`ai::Brain::wants_breach`).
        enemy_breach_after_seconds: f32 = 0.5 in 0.05 ..= 5.0;
        /// A breach is abandoned after this long without the tile falling.
        enemy_breach_give_up_seconds: f32 = 5.0 in 0.5 ..= 30.0;
        /// A shell-armed tank only breaches with at least this many shells
        /// left, so it still has a fight in it afterwards ...
        enemy_breach_min_shells: i32 = 4 in 0 ..= 100;
        /// ... and with damage at or under this (0 pristine, 100 wreck).
        enemy_breach_max_damage: f32 = 60.0 in 0.0 ..= 100.0;
        /// Seconds between breach shots - quicker than the combat cadence,
        /// a wall doesn't shoot back.
        enemy_breach_fire_interval: f32 = 0.45 in 0.05 ..= 5.0;
        /// How far past its own hull a tank looks for the tile it is
        /// driving into (px): about one tile.
        enemy_breach_reach_px: f32 = 40.0 in 4.0 ..= 300.0;
    }

    group command {
        /// The switch on the enemy command & control layer
        /// (docs/enemy-command-and-control-prd.md). False is a **total**
        /// no-op: the commander is not consulted, no intent is rewritten and
        /// no order is issued - not a mitigation of strength zero. That is
        /// what lets a build with C2 off reproduce a pre-C2 round bit for
        /// bit, which is the A/B this whole feature is measured by, and it is
        /// why this is a bool rather than a strength.
        ///
        /// Safe to flip mid-round: tuning writes are staged and applied at
        /// the frame boundary. Reachable from the dev panel, `--tuning`,
        /// `--no-c2` and the dev-build key toggle.
        /// **Off by default, deliberately, and not because it does not
        /// work.** As measured (docs/enemy-command-and-control-prd.md
        /// section 10) the arbiter is a large win in open ground - 21
        /// enemy-vs-enemy rams down to 6 on a bare map over 16 seeds - and
        /// roughly neutral on the cluttered shipped map, where it trades a
        /// 15% cut in enemy-vs-enemy ramming for three regressions it has no
        /// answer for yet: more rams into the *player* (enemies that stop
        /// shoving each other arrive at you more successfully), and a
        /// handful of `stall`/`low-progress` flags where a yielding tank
        /// waits out a jam it used to barge through.
        ///
        /// Flipping this default is Phase 3's job: the ram authorisation
        /// gate is what addresses player rams, and it is meaningless to
        /// judge the arbiter's effect on them until that exists. Until then
        /// the shipped game is unchanged and this is the switch to play
        /// with - `--no-c2`, the dev-build key, the tuning panel, or
        /// `tuning_set` over MCP.
        c2_enabled: bool = false in 0 ..= 1;
        /// Seconds ahead the commander predicts hull contact. Deliberately
        /// shorter than `avoid_lookahead` (0.8) so the two layers own
        /// disjoint time bands: the AI's predictive sidestep has already had
        /// its chance to solve a crossing, and this is the late window where
        /// it has either declined or failed.
        c2_horizon_seconds: f32 = 0.5 in 0.0 ..= 3.0;
        /// Clearance beyond the two hull radii (px) that counts as contact.
        /// Under `avoid_margin` (12) on purpose - `Ai::avoid_collisions`
        /// *skips* every pair already inside that, so triggering below it is
        /// what keeps the two from arguing over the same tanks.
        c2_contact_margin_px: f32 = 6.0 in 0.0 ..= 60.0;
        /// Centre-distance cull (px) before the pair test. The O(n^2) scan's
        /// only optimisation, and with at most 33 movers its only needed one.
        c2_watch_px: f32 = 200.0 in 50.0 ..= 1000.0;
        /// Real closing speed (px/s) below which a pair is resting against
        /// each other rather than colliding - the stuck escape's business,
        /// not the commander's. Above solver jitter and above
        /// `stuck_speed_eps`.
        c2_min_closing_px: f32 = 20.0 in 0.0 ..= 200.0;
        /// Slowest a throttled tank is driven, as a fraction of its speed.
        /// Above zero on purpose: a throttled tank is still *commanded* to
        /// move, so `Ai`'s stuck clock keeps running underneath and
        /// `stuck_escape_seconds` stays armed - which a full brake
        /// (`move_dir = None`) resets every frame, and which is why
        /// `enemy_yield_seconds` has to be capped at all.
        c2_throttle_floor: f32 = 0.35 in 0.0 ..= 1.0;
        /// Speed quantisation (px/s) in the right-of-way comparator, so a
        /// priority flip needs a real difference rather than float noise.
        c2_speed_bucket_px: f32 = 40.0 in 1.0 ..= 200.0;
        /// Ceiling on one yield. Past it the tank drives on regardless, the
        /// same safety valve `enemy_yield_seconds` provides one level down:
        /// without a ceiling a pair that cannot resolve would hold forever.
        c2_yield_hold_seconds: f32 = 1.5 in 0.0 ..= 10.0;
        /// How long a pair must stay in conflict before the commander
        /// escalates from easing off to stopping.
        c2_escalate_seconds: f32 = 0.5 in 0.0 ..= 5.0;
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

    group field {
        /// Field maps only (a map the camera follows - bigger than an
        /// arena or `view = "follow"`; docs/large-maps-follow-camera.md
        /// section 12, `simulation::field`): an enemy that sees a seat
        /// alerts every enemy within this many px of itself, and they
        /// pass it on the same way, so an alert travels down a chain of
        /// neighbours instead of reaching the whole map. Like the
        /// arena's shared alert, a pure distance test with no line of
        /// sight. As far as an enemy sees in daylight
        /// (`enemy_view_range`): a tank alerts the ones it could see, so
        /// a sighting runs through a group of neighbours, and across a map
        /// several screens wide only as far as its enemies stand that
        /// close to one another.
        enemy_alert_chain_px: f32 = 800.0 in 0.0 ..= 4000.0;
        /// Field maps only: how far from home (where it spawned, or came
        /// through its gate) an enemy with nothing to fight may roam. Past
        /// it, an enemy with no alert, no call to the fight and no target
        /// in sight turns back home, and it wanders and seeks pickups only
        /// inside it.
        enemy_leash_px: f32 = 640.0 in 32.0 ..= 4000.0;
        /// Field maps only: an enemy farther than this from every live
        /// seat and the players' frog is far. A far enemy thinks only every
        /// `enemy_far_think_ticks` ticks, and one nothing has woken yet -
        /// no alert, no hit, no call, no seat this close - does not think
        /// or route at all. Past `enemy_view_range`, so a far enemy cannot
        /// see anyone to fight.
        enemy_far_px: f32 = 1200.0 in 100.0 ..= 8000.0;
        /// Field maps only: a far enemy thinks once every this many ticks,
        /// staggered by owner slot, and keeps driving its last intent
        /// (never its trigger) on the ticks between. 1 thinks every tick.
        enemy_far_think_ticks: usize = 4 in 1 ..= 60;
        /// Field maps only: the walk to the fight a spawn or a wave gate
        /// aims for, in seconds of path from the nearest seat at
        /// `enemy_speed` - out of sight, within reach. Of the gates
        /// outside every seat's sight box a wave takes the ones whose walk
        /// is within `field_walk_slack_seconds` of this, where the map has
        /// any, and a band spawn's draws lean toward such cells; a map with
        /// no walk that long (one about 40 cells across) only keeps its
        /// spawns out of sight.
        field_walk_seconds: f32 = 15.0 in 1.0 ..= 120.0;
        /// The window either side of `field_walk_seconds`: wide enough
        /// that a wave still spreads over several lanes and a band over a
        /// region rather than one ring of cells.
        field_walk_slack_seconds: f32 = 5.0 in 0.0 ..= 60.0;
        /// How many of those cells a band spawn draws, keeping whichever
        /// stands farthest from the enemies already down, so a band
        /// spreads over its region rather than starting in a heap.
        field_spawn_spread_candidates: u32 = 6 in 1 ..= 32;
        /// Field maps' wave rounds only: stragglers are rolled in again
        /// (`Game::reroll_stragglers`) - a wave tank that has gone
        /// `field_reroll_after_seconds` without a live seat or the players'
        /// frog in its sight, and stands farther from them by walk than
        /// any wave's gate is paced for (`field_walk_seconds` less
        /// `field_walk_slack_seconds`), is taken off where no screen can
        /// see it go and rolls in again through a gate nearer the fight,
        /// outside every sight box. Off, a straggler walks on as it is.
        field_reroll: bool = true in 0 ..= 1;
        /// How long a wave tank goes without a seat or the frog in its
        /// sight before it counts as a straggler: long past a wave's walk
        /// to the fight, so only one that lost its way - through a portal
        /// for a pickup, its call over, home on its leash - is taken.
        field_reroll_after_seconds: f32 = 30.0 in 1.0 ..= 600.0;
    }

    group director {
        /// Field maps' wave rounds only: the pacing director
        /// (`simulation::director`, docs/large-maps-follow-camera.md
        /// section 12) paces the breather before each wave by how hard
        /// the team is pressed - held while it is at its peak, stretched
        /// to a rest after one, shortened while nothing happens. Off, a
        /// field map's breather is `wave_gap_seconds`, as an arena's
        /// always is.
        director_enabled: bool = true in 0 ..= 1;
        /// A seat's intensity (0 to 1) at or above this is the team at its
        /// peak: the next wave waits, and a rest is owed once it passes.
        director_peak: f32 = 0.8 in 0.05 ..= 1.0;
        /// At or under this the team is calm - nothing is happening - and
        /// with no rest owed the breather runs `director_calm_rate` times
        /// as fast.
        director_calm: f32 = 0.2 in 0.0 ..= 1.0;
        /// The share of a pool lost at once that takes a seat from calm to
        /// the top: its tank's health and shield (100 points), or the
        /// players' frog's health (`frog_max_health`), which jolts every
        /// seat. 0.35 is three or four enemy shells on the tank, one or two
        /// on the frog's 40.
        director_hurt_full: f32 = 0.35 in 0.01 ..= 4.0;
        /// Live enemies inside a seat's sight box that hold its intensity
        /// at the top; fewer hold it at their share.
        director_crowd_full: f32 = 3.0 in 0.5 ..= 31.0;
        /// Seconds a seat's intensity takes to fall from the top to
        /// nothing once nothing jolts it and its box is empty.
        director_fall_seconds: f32 = 10.0 in 0.1 ..= 120.0;
        /// The rest a peak owes the team once it passes, in seconds: no
        /// breather ends sooner after one.
        director_relax_seconds: f32 = 12.0 in 0.0 ..= 120.0;
        /// How many times as fast a breather runs down while the team is
        /// calm and no rest is owed.
        director_calm_rate: f32 = 2.0 in 1.0 ..= 10.0;
        /// The shortest a breather may be, calm or not ...
        director_breather_min_seconds: f32 = 2.0 in 0.0 ..= 60.0;
        /// ... and the longest, holds included: past it the next wave
        /// comes whatever the team is doing, so a round always goes on.
        director_breather_max_seconds: f32 = 30.0 in 0.0 ..= 300.0;
    }

    group portal {
        /// A tank whose centre comes this close (px) to a portal's anchor
        /// centre teleports (docs/teleporting.md). Also the portal's nav
        /// footprint: every grid cell whose centre is within it is a cell
        /// the AI may route into the hub from.
        portal_trigger_radius: f32 = 40.0 in 8.0 ..= 200.0;
        /// Seconds after arriving before a tank may enter any portal
        /// again - the arrival cell is just outside the exit's trigger
        /// radius, so without this a tank rolling on would bounce back.
        /// Counts down only while the tank is off every portal: one that
        /// parks on its exit to fight never re-triggers until it leaves.
        portal_cooldown_seconds: f32 = 1.5 in 0.0 ..= 30.0;
        /// What the AI's planner charges for a hop, in grid cells. Low
        /// makes enemies take every portal that shortens the walk; high
        /// makes them walk unless the portal saves a whole trip. Never
        /// below one step, or the hub would beat walking between two
        /// cells of the same footprint (`Grid::with_portals` clamps).
        portal_hop_cost: f32 = 4.0 in 1.0 ..= 64.0;
        /// How many grid steps out from the exit portal the arrival
        /// search may walk (through open cells only) for a free cell
        /// outside the exit's trigger radius. Nothing free within it means
        /// the teleport does not happen this frame.
        portal_arrival_max_cells: i32 = 6 in 1 ..= 32;
        /// Whether shells, bullets, plasma bolts and laser beams pass
        /// through portals too (docs/teleporting.md, "Shots"). Off, a
        /// shot flies over a portal as over open ground.
        portal_shots: bool = true in 0 ..= 1;
        /// A shot whose path passes this close (px) to a portal's anchor
        /// goes in, at the point of its path nearest the anchor, and comes
        /// out of another portal at the same offset from its anchor,
        /// heading and speed kept. Smaller than a tank's trigger: a shot is
        /// a point, and should have to hit the swirl rather than graze the
        /// rim.
        portal_shot_radius: f32 = 28.0 in 4.0 ..= 120.0;
        /// How many portals one shot or beam may pass through; past it the
        /// portals let it fly over. A shot lined up between two portals
        /// would otherwise loop for ever.
        portal_shot_max_passes: i32 = 4 in 0 ..= 32;
    }

    group volcano {
        /// The volcano's cycle (docs/volcano.md): asleep, a rumble that
        /// warns, the eruption, then the cooling - a pure function of the
        /// round clock (`volcano::Phase`), so a replica rumbles on the
        /// room's tick with nothing on the wire. The whole cycle (seconds).
        volcano_period_seconds: f32 = 30.0 in 8.0 ..= 300.0;
        /// The rumble before each eruption: the crater brightens, the
        /// plume darkens, the ground trembles and the off-screen arrow
        /// pulses (seconds).
        volcano_rumble_seconds: f32 = 4.0 in 0.5 ..= 30.0;
        /// The eruption: the fountain, the shock ring and the lava bombs
        /// (seconds).
        volcano_erupt_seconds: f32 = 5.0 in 0.5 ..= 30.0;
        /// The cooling after it: the rivers' surge dies down (seconds).
        volcano_cool_seconds: f32 = 7.0 in 0.5 ..= 60.0;
        /// When the first rumble starts on the round clock (seconds); each
        /// volcano on a map is offset from it by a hash of its cell.
        volcano_first_rumble_seconds: f32 = 14.0 in 0.0 ..= 300.0;
        /// Lava bombs an eruption throws, spread over its length.
        volcano_bombs_per_eruption: i32 = 9 in 0 ..= 64;
        /// How far from the crater a bomb lands (px): no nearer than the
        /// first, no further than the second.
        volcano_bomb_min_range_px: f32 = 120.0 in 32.0 ..= 2000.0;
        volcano_bomb_range_px: f32 = 460.0 in 64.0 ..= 4000.0;
        /// The share of an eruption's bombs thrown at a seat within range
        /// rather than at a spot round the crater. An aimed bomb lands a
        /// hashed step off its seat, always inside that seat's sight box,
        /// so nothing lands on a player from a screen they were never shown.
        volcano_bomb_aimed_share: f32 = 0.45 in 0.0 ..= 1.0;
        /// A bomb's flight from the crater to where it lands (seconds) -
        /// the warning a player has, its ring on the ground the whole way -
        /// and how high it arcs (px).
        volcano_bomb_flight_seconds: f32 = 1.9 in 0.3 ..= 6.0;
        volcano_bomb_arc_px: f32 = 150.0 in 0.0 ..= 600.0;
        /// A landing bomb's blast: its radius (px), its damage at the
        /// centre (falling to nothing at the radius) and its shove. It
        /// hurts both sides and every tile it reaches, like a drum.
        volcano_bomb_radius_px: f32 = 58.0 in 8.0 ..= 300.0;
        volcano_bomb_damage_min: f32 = 16.0 in 0.0 ..= 100.0;
        volcano_bomb_damage_max: f32 = 28.0 in 0.0 ..= 100.0;
        volcano_bomb_knockback: f32 = 140.0 in 0.0 ..= 1000.0;
        /// The lava a bomb splashes: the cells it sets burning round where
        /// it lands (radius, cells) and how long they burn (seconds).
        volcano_pool_radius_cells: i32 = 1 in 0 ..= 4;
        volcano_pool_seconds: f32 = 6.0 in 0.5 ..= 60.0;
        /// The eruption's shock ring and shake, as a fraction of a kill's.
        volcano_shock_scale: f32 = 1.6 in 0.0 ..= 4.0;
        /// A lava stream (a ford): the fraction of top speed and of grip a
        /// hull keeps wading it.
        lava_speed_factor: f32 = 0.5 in 0.05 ..= 1.0;
        lava_grip_factor: f32 = 0.7 in 0.05 ..= 1.0;
        /// What the AI's router charges for a step into a lava ford, in
        /// grid cells: high enough that an enemy walks round a stream where
        /// a bridge is anywhere near.
        lava_ford_path_cost: i32 = 14 in 1 ..= 64 @ Restart;
        /// Damage per second at full heat - in the lava itself. A bank
        /// burns by how far its heat stands over `heat_hurt_from`.
        lava_damage_per_second: f32 = 24.0 in 0.0 ..= 200.0;
        /// Heat at and under this hurts nothing - the scorched ground
        /// only.
        heat_hurt_from: f32 = 0.5 in 0.0 ..= 1.0;
        /// How long a hull keeps burning after it leaves the lava
        /// (seconds; the flamethrower's afterburn, `flame_afterburn_dps`).
        lava_afterburn_seconds: f32 = 1.5 in 0.0 ..= 10.0;
        /// How fast heat falls off a cell at a time away from the lava:
        /// each cell out keeps this fraction of the last's.
        lava_heat_falloff: f32 = 0.55 in 0.05 ..= 0.95 @ Restart;
        /// How fast the lava's bands run downstream (px/s); the surge of an
        /// eruption runs them up to twice as fast.
        lava_flow_speed: f32 = 9.0 in 0.0 ..= 60.0;
        /// Motes off the lava: sparks a second off each cell, three times
        /// as many in a surge (the particle layer's, `fx.rs`).
        lava_mote_rate: f32 = 0.35 in 0.0 ..= 10.0;
        /// The heat shield pickup: how long it keeps every kind of heat off
        /// its tank (seconds).
        heat_shield_seconds: f32 = 10.0 in 0.5 ..= 60.0;
        /// A lamp post's health: one hit puts it out at the default.
        lamp_max_health: f32 = 1.0 in 0.1 ..= 500.0;
        /// How far a lamp's light carries at night (px).
        lamp_light_px: f32 = 150.0 in 16.0 ..= 600.0;
        /// Whoever stands this close to a lamp (px) - or on ground at
        /// least `lava_reveal_heat` hot, where the lava lights them - an
        /// enemy sees at full `enemy_view_range` however dark the sky:
        /// light cuts both ways.
        lamp_reveal_px: f32 = 110.0 in 0.0 ..= 600.0;
        lava_reveal_heat: f32 = 0.3 in 0.0 ..= 1.0;
        /// Lanterns each seat may set down in a round, when the round's
        /// sky is dark or night will fall in it.
        lamps_per_seat: i32 = 3 in 0 ..= 16;
        /// How long the dark takes to fall before a map's `nightfall`
        /// (seconds): the light eases from the map's sky into night's, and
        /// the rules turn at `nightfall` itself.
        nightfall_seconds: f32 = 25.0 in 0.0 ..= 300.0;
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
        /// Bite a tank of the *other* side within this factor of the
        /// frog's size (`frog::Side::bites`: a frog never bites the side
        /// whose objective it is).
        frog_attack_range_factor: f32 = 0.9 in 0.0 ..= 5.0;
        frog_attack_cooldown_seconds: f32 = 1.5 in 0.0 ..= 10.0;
        frog_attack_damage_min: f32 = 4.0 in 0.0 ..= 100.0;
        frog_attack_damage_max: f32 = 10.0 in 0.0 ..= 100.0;
        /// Personal space: hop away from the nearest tank of *either* side
        /// once one gets within this factor of the frog's size - larger
        /// than the bite range ("keep your distance" rather than
        /// "retaliate"), well inside the hop distance so a single hop
        /// reliably clears it.
        frog_avoid_range_factor: f32 = 1.2 in 0.0 ..= 5.0;
    }

    group walls {
        /// Per-material toughness: hp absorbed before reaching the terminal
        /// state (rubble/charred/shattered), or - for Iron - before its rust
        /// stage plateaus (Iron is never destroyed). Ordered fragile to
        /// tough: glass snaps almost immediately, wood breaks easily, brick
        /// holds longer, iron the longest of all on top of being permanent.
        /// Baked into each wall's health when the map is spawned.
        wall_max_health: [f32; 4] = [20.0, 220.0, 8.0, 2.0] in 1.0 ..= 1000.0 labels MATERIAL_NAMES @ Spawn;
        /// Fraction of spawned Wood obstacles that catch fire when destroyed
        /// instead of breaking outright (`Obstacle::flammable`, rolled once
        /// at spawn).
        wood_flammable_chance: f64 = 0.7 in 0.0 ..= 1.0 @ Spawn;
        /// Burning wood's 3-frame flicker loop cadence (~7.7 FPS at 0.13).
        wood_burn_frame_seconds: f32 = 0.395 in 0.01 ..= 2.0;
        /// Total time a Wood obstacle spends burning before charring and
        /// being removed.
        wood_burn_seconds: f32 = 1.0 in 0.1 ..= 30.0;
    }

    group props {
        /// Sandbag toughness: hp absorbed over its three visible stages
        /// (intact, torn, collapsed) before it flattens. Baked in at spawn.
        sandbag_max_health: f32 = 45.0 in 1.0 ..= 500.0 @ Spawn;
        /// Odds a shell, bullet or plasma bolt sails over a sandbag tile
        /// instead of hitting it, rolled per projectile per tile.
        sandbag_pass_over_chance: f64 = 0.35 in 0.0 ..= 1.0;
        /// Seconds a tank has to keep pushing into a sandbag before it
        /// collapses. Kept under `enemy_breach_after_seconds` so an AI that
        /// drives into one pushes through instead of stopping to shoot it.
        sandbag_ram_seconds: f32 = 0.4 in 0.05 ..= 5.0;
        /// Barrel toughness before it detonates: a player shell usually pops
        /// it outright, an enemy shell needs two, minigun bullets several.
        /// Baked in at spawn.
        barrel_max_health: f32 = 18.0 in 1.0 ..= 500.0 @ Spawn;
        /// Odds a projectile flies over a barrel instead of hitting it.
        barrel_pass_over_chance: f64 = 0.08 in 0.0 ..= 1.0;
        /// Odds a shell or bullet ricochets off a barrel instead of hitting
        /// it (plasma never ricochets).
        barrel_deflect_chance: f64 = 0.1 in 0.0 ..= 1.0;
        /// Delay between chain-reaction links: a barrel caught in another
        /// blast is armed for about this long before it goes off itself
        /// (half of it at the blast's centre, two and a half times at its edge),
        /// so a cluster cascades outward instead of vanishing in one frame.
        barrel_fuse_seconds: f32 = 0.18 in 0.0 ..= 2.0;
        /// Blast radius (px) of a detonating barrel: everything inside takes
        /// damage with linear falloff, and other barrels inside always chain.
        barrel_blast_radius: f32 = 96.0 in 0.0 ..= 600.0;
        /// Blast damage at the centre, rolled per victim; falls off linearly
        /// to zero at the radius. Hurts everyone: player, enemies, frogs,
        /// walls and props alike.
        barrel_blast_damage_min: f32 = 15.0 in 0.0 ..= 100.0;
        /// Upper end of the blast damage roll.
        barrel_blast_damage_max: f32 = 30.0 in 0.0 ..= 100.0;
        /// Outward shove a barrel blast gives a tank at its centre (px/s,
        /// mass-normalised like a wreck's knockback).
        barrel_blast_knockback_speed: f32 = 140.0 in 0.0 ..= 500.0;
        /// Damage per second a tank pushing into a barrel deals it - so
        /// ramming one sets it off in a fraction of a second.
        barrel_ram_damage_per_second: f32 = 40.0 in 0.0 ..= 500.0;
        /// Extra shove the blast gives a tank that set a barrel off by
        /// driving into it, as a multiple of the ordinary knockback: the
        /// hull is sitting on the drum, so it should visibly lurch.
        barrel_ram_kick_factor: f32 = 2.2 in 1.0 ..= 6.0;
        /// Per-blast pace jitter on the fireball (`blast_fireball_seconds`),
        /// as a fraction either way, hashed from the blast position: two
        /// adjacent blasts then never burn and cool in lockstep, which is
        /// the biggest "cloned" tell a cascade has.
        barrel_fps_jitter: f32 = 0.15 in 0.0 ..= 0.5;
        /// Per-blast size jitter on the fireball (`blast_fireball_px`), same
        /// idea.
        barrel_scale_jitter: f32 = 0.1 in 0.0 ..= 0.5;
        /// Most delayed secondary pops a barrel blast queues (the same
        /// cook-offs a wreck gets); the count is hashed per blast from
        /// 0 to this, so a third of drums get none.
        barrel_cookoff_max: i32 = 2 in 0 ..= 8;
        /// Drum parts (a lid and staves) a blast throws in arcs to hashed
        /// landing spots. Zero leaves one static rubble decal in place
        /// instead.
        barrel_parts: i32 = 3 in 0 ..= 12;
        /// How far those parts scatter (px).
        barrel_part_throw_px: f32 = 60.0 in 0.0 ..= 300.0;
        /// Fraction of the blast radius inside which tall grass is
        /// flattened (the same `crush` a hull drives).
        blast_grass_flatten: f32 = 0.6 in 0.0 ..= 1.5;
        /// Most landed rubble pieces one blast picks up and throws again
        /// (the scene reacts to a second blast in the same place).
        blast_rethrow_max: i32 = 8 in 0 ..= 32;
        /// Sideways rock (px, whole 2px blocks) of a drum whose fuse is
        /// lit; 0 keeps it still.
        barrel_fuse_rock_px: f32 = 2.0 in 0.0 ..= 8.0;
        /// Sparks a second thrown from the bung of a fused drum.
        barrel_fuse_spark_rate: f32 = 14.0 in 0.0 ..= 60.0;
        /// Fuse length multiplier for the red oil drum: it smoulders, so
        /// the lit lid, the rocking and the sparks get time to read.
        oil_fuse_factor: f32 = 3.0 in 0.1 ..= 10.0;
        /// Fuse length multiplier for the grey fuel drum: it cracks first.
        fuel_fuse_factor: f32 = 0.5 in 0.1 ..= 10.0;
        /// Cells around an oil drum's blast that burn afterwards: 0 for
        /// none, 1 for the centre plus its four neighbours.
        oil_pool_radius_cells: i32 = 1 in 0 ..= 3;
        /// How long the pool burns.
        oil_pool_seconds: f32 = 4.0 in 0.1 ..= 30.0;
        /// Damage per second to a tank whose hull is over a burning cell.
        oil_pool_damage_per_second: f32 = 6.0 in 0.0 ..= 100.0;
        /// Odds an oil drum leaves a pool at all. 1.0 and 0.0 draw no RNG.
        oil_pool_chance: f64 = 1.0 in 0.0 ..= 1.0;
        /// How fast a lit oil trail (`kind = "oil"` cells) runs along its
        /// length, in cells per second.
        oil_trail_cells_per_second: f32 = 6.0 in 0.5 ..= 30.0;
        /// How long one trail cell burns once the fire reaches it.
        oil_trail_burn_seconds: f32 = 2.5 in 0.1 ..= 30.0;
        /// Fuse a drum sitting in a burning cell gets, as a multiple of
        /// `barrel_fuse_seconds` (before the drum's own kind factor).
        fire_fuse_factor: f32 = 2.0 in 0.1 ..= 10.0;
        /// The grey fuel drum's blast: bigger, sharper and shorter than
        /// the oil drum's, with no pool.
        fuel_blast_radius: f32 = 128.0 in 0.0 ..= 600.0;
        fuel_blast_damage_min: f32 = 20.0 in 0.0 ..= 100.0;
        fuel_blast_damage_max: f32 = 40.0 in 0.0 ..= 100.0;
        fuel_blast_knockback_speed: f32 = 220.0 in 0.0 ..= 500.0;
        /// Odds a fuel drum set off by *another blast* launches - flies
        /// two or three cells away from the source and detonates where it
        /// lands - instead of popping in place. 1.0 and 0.0 draw no RNG.
        fuel_launch_chance: f64 = 1.0 in 0.0 ..= 1.0;
        /// Shortest and longest launch, in cells; the length is hashed
        /// per drum between the two.
        fuel_launch_cells_min: i32 = 2 in 1 ..= 8;
        fuel_launch_cells_max: i32 = 3 in 1 ..= 8;
        /// Scale of the scorch a fuel drum leaves, next to an oil drum's 1.
        scorch_fuel_scale: f32 = 1.3 in 0.5 ..= 3.0;
        /// Odds a hit on a pristine fence destroys it outright; otherwise it
        /// drops to its damaged keyframe and the next hit finishes it.
        fence_one_shot_chance: f64 = 0.7 in 0.0 ..= 1.0;
        /// Seconds a tank has to push into a fence before it gives way.
        fence_ram_seconds: f32 = 0.15 in 0.05 ..= 5.0;
        /// Broadleaf toughness: hp absorbed over its three visible stages
        /// (full crown, thinning, nearly bare) before it comes down.
        ///
        /// **Deliberately brittle, in the fence bracket rather than the
        /// wall bracket.** Set from the *weakest* chassis: a player shell
        /// rolls `player_damage_min..max` scaled by `tank_damage_factor`,
        /// so a scout does 7.5-22.5 and clears 12 about 70% of the time -
        /// the same one-shot rate `fence_one_shot_chance` gives a fence,
        /// arrived at through damage instead of a coin flip. Heavier
        /// chassis fell one every time, a minigun burst still has to chew
        /// through it, and a barrel blast still takes a whole stand down.
        /// Baked in at spawn.
        tree_max_health: f32 = 12.0 in 1.0 ..= 500.0 @ Spawn;
        /// Conifer toughness. Slimmer than a broadleaf, so a little less.
        pine_max_health: f32 = 9.0 in 1.0 ..= 500.0 @ Spawn;
        /// Seconds a tank has to keep pushing into a tree before it goes
        /// over. Just past a sandbag's, so a tank drives through a stand
        /// rather than shouldering each trunk down, and well short of
        /// `enemy_breach_after_seconds` so an AI that drives into one
        /// pushes through instead of stopping to shoot it.
        tree_ram_seconds: f32 = 0.5 in 0.05 ..= 10.0;
        /// Seconds each of a tree's dapple frames holds.
        ///
        /// This is a tree's entire idle animation: the frames are the same
        /// canopy with a few patches of leaf one rung lighter, drifting
        /// between frames, so light moves through the crown and the crown
        /// itself does not. An ambient *bend* was built first and rejected -
        /// what looks alive on a blade of grass looks wrong on a trunk. Each
        /// tree takes its phase from a hash of its position, so a wood
        /// shimmers out of step with itself rather than blinking as one
        /// (`obstacle::tree_col`).
        tree_dapple_seconds: f32 = 0.9 in 0.05 ..= 10.0;
        /// How far a tree bends away from a tank shouldering it over, at
        /// the moment it goes. Scales with `ram_timer` squared, so the tree
        /// gives slowly at first and then goes - without it a rammed tree
        /// stands bolt upright until it simply vanishes.
        tree_lean_px: f32 = 22.0 in 0.0 ..= 80.0;
        /// Clear gap (px, hull surface to hull surface) a tank tries to keep
        /// from whatever is directly in front of it. Inside this it stops
        /// driving rather than pressing on, so tanks converging on the same
        /// place pull up short instead of slamming into each other and into
        /// the player. Separate from `avoid_margin`, which feeds the
        /// *predictive sidestep* (`Ai::avoid_collisions`) and is about
        /// paths that will cross later; this one is about the tank already
        /// in the way now.
        enemy_separation_px: f32 = 12.0 in 0.0 ..= 200.0;
        /// How long a tank will sit yielding before it gives up and drives
        /// on anyway. Without a ceiling two tanks nose to nose both brake
        /// and neither ever moves again - the stuck escape cannot save them
        /// either, because it only counts tanks that were *commanded* to
        /// move. Past this the brake releases and behaviour is exactly what
        /// it was before, so the worst case is a visible pause rather than
        /// a freeze.
        ///
        /// The timer runs for as long as the way ahead stays blocked and
        /// resets only when it clears, so a tank holds *once* and then
        /// drives on. Decaying it while still blocked instead makes the
        /// tank brake, release, fall back under the ceiling and brake
        /// again - a permanent half-speed shuffle rather than a yield.
        enemy_yield_seconds: f32 = 0.5 in 0.0 ..= 20.0;
        /// Odds a tree is the kind that catches fire when it dies instead
        /// of simply falling, rolled once per tile at spawn. Zero draws no
        /// RNG at all, so a treeless map replays unchanged. Burn timing is
        /// shared with wood (`wood_burn_seconds`): it is one fire.
        tree_flammable_chance: f64 = 0.55 in 0.0 ..= 1.0 @ Spawn;
        /// Radius (px) of a blast's fireball at its peak (`fireball.rs`),
        /// before the blast's own scale - a fuel drum's is larger, a
        /// missile's and a cook-off's smaller.
        blast_fireball_px: f32 = 34.0 in 4.0 ..= 160.0;
        /// How long a blast's fire and smoke play (seconds) before the
        /// blast's own pace: the fire burns out over about the first third
        /// and the smoke climbs and thins away over the rest.
        blast_fireball_seconds: f32 = 1.7 in 0.2 ..= 8.0;
        /// The flames a burning ground cell stands in (`pyro::tongues`):
        /// how many tongues, how tall (px) the tallest, and how far (px)
        /// across the cell they spread.
        ground_fire_tongues: i32 = 3 in 0 ..= 8;
        ground_fire_height_px: f32 = 28.0 in 2.0 ..= 80.0;
        ground_fire_spread_px: f32 = 24.0 in 0.0 ..= 64.0;
        /// How long the light bloom a blast opens with lasts.
        blast_glow_seconds: f32 = 0.25 in 0.0 ..= 2.0;
        /// Radius (px) of that bloom at its largest.
        blast_glow_radius: f32 = 64.0 in 0.0 ..= 400.0;
        /// Peak opacity of the bloom.
        blast_glow_strength: f32 = 0.45 in 0.0 ..= 1.0;
        /// Peak opacity of the whole-screen flash a blast starts with.
        blast_screen_flash_alpha: f32 = 0.12 in 0.0 ..= 1.0;
        /// How long that screen flash takes to fade.
        blast_screen_flash_seconds: f32 = 0.06 in 0.0 ..= 0.5;
        /// Minimum spacing between two whole-screen flashes, so a barrel
        /// chain or a multi-kill reads as one flash rather than a strobe.
        blast_screen_flash_min_gap_seconds: f32 = 0.35 in 0.0 ..= 2.0;
        /// Opacity of the pulsing glow on a barrel whose fuse is lit.
        barrel_fuse_glow_strength: f32 = 0.6 in 0.0 ..= 1.0;
        /// Opacity of the burn mark a blast leaves on the ground.
        scorch_opacity: f32 = 0.75 in 0.0 ..= 1.0;
        /// Seconds a fresh scorch mark takes to fade in under the fireball.
        scorch_fade_in_seconds: f32 = 0.25 in 0.0 ..= 2.0;
        /// On-screen scale of the 64px scorch decal cells.
        scorch_scale: f32 = 2.0 in 0.5 ..= 4.0;
        /// Opacity of the rubble a destroyed wall tile leaves behind
        /// (`decal::Decal`). Below the tile it replaces, so a levelled
        /// wall reads as ground the tank can drive over rather than as a
        /// wall that stopped being solid.
        decal_opacity: f32 = 0.45 in 0.0 ..= 1.0;
        /// Seconds fresh rubble takes to fade in, so a tile doesn't snap
        /// straight from standing to wreckage.
        decal_fade_in_seconds: f32 = 0.18 in 0.0 ..= 2.0;
        /// Seconds a blown-off part spends in the air before settling into
        /// the landing spot the simulation already picked for it.
        debris_flight_seconds: f32 = 0.5 in 0.05 ..= 3.0;
        /// Peak height (px) of that arc, varied per piece by its position
        /// hash. Not real height - the game is top-down, so this is a draw
        /// offset plus a shrinking shadow.
        debris_arc_height: f32 = 40.0 in 0.0 ..= 200.0;
        /// Parts a dying tank throws, and how far they scatter.
        wreck_parts: i32 = 5 in 0 ..= 32;
        wreck_part_throw_px: f32 = 64.0 in 0.0 ..= 400.0;
        /// Delayed secondary pops after a tank dies (ammo cooking off):
        /// how many, spread over how long, and how big each fireball is
        /// next to the main one.
        cookoff_count: i32 = 2 in 0 ..= 12;
        cookoff_window_seconds: f32 = 1.6 in 0.1 ..= 10.0;
        cookoff_blast_scale: f32 = 0.45 in 0.1 ..= 2.0;
        /// Share of tank deaths that go up as the mushroom cloud (a stem
        /// of fire under a rolling cap) instead of a plain fireball.
        /// Picked from the kill position's hash, so no RNG is drawn.
        wreck_mushroom_chance: f32 = 0.7 in 0.0 ..= 1.0;
        /// The mushroom cloud's life in seconds, its height (px the cap
        /// climbs above the hull) and its cap's radius (px) - each
        /// jittered per kill by the position hash (`mushroom::Cloud`).
        mushroom_seconds: f32 = 2.8 in 0.5 ..= 8.0;
        mushroom_height_px: f32 = 84.0 in 16.0 ..= 240.0;
        mushroom_cap_px: f32 = 30.0 in 8.0 ..= 96.0;
        /// A dying tank burns its last tread marks into the ground: this
        /// many of them stop fading and darken by this multiple, so the
        /// kill site stays readable after the wreck is cleared.
        wreck_track_marks: i32 = 10 in 0 ..= 64;
        wreck_track_darken: f32 = 1.8 in 1.0 ..= 4.0;

        // --- the short-lived particle layer (fx.rs) ---
        /// Global multiplier on every particle count. `main.rs` starts the
        /// web build lower: the wasm target is the tighter budget, and a
        /// dense wave is where that shows.
        fx_density: f32 = 1.0 in 0.0 ..= 3.0;
        /// Hard cap on live particles; oldest are evicted first so a big
        /// burst eats into old smoke rather than into itself.
        fx_max_particles: i32 = 900 in 0 ..= 8000;
        /// Downward acceleration on a chip's fake height, px/s^2.
        debris_gravity: f32 = 900.0 in 0.0 ..= 4000.0;
        /// Per-second rate at which a particle bleeds ground speed
        /// (exponential, so it is frame-rate independent).
        debris_air_drag: f32 = 2.4 in 0.0 ..= 20.0;
        /// Fraction of vertical speed a chip keeps per ground bounce.
        debris_bounce: f32 = 0.35 in 0.0 ..= 1.0;
        spark_lifetime: f32 = 0.4 in 0.05 ..= 5.0;
        chip_lifetime: f32 = 1.1 in 0.05 ..= 5.0;
        dust_lifetime: f32 = 0.8 in 0.05 ..= 5.0;
        smoke_lifetime: f32 = 2.2 in 0.05 ..= 20.0;
        ember_lifetime: f32 = 0.9 in 0.05 ..= 10.0;
        /// Upward drift of smoke and embers, and how fast a smoke puff
        /// grows as it rises.
        smoke_rise_speed: f32 = 26.0 in 0.0 ..= 200.0;
        /// How far smoke, embers, dust and the smoke of every explosion
        /// drift down-wind for each px they rise, in a typical gust: the
        /// wind that bends the tall grass (`pyro::smoke_lean`,
        /// `grass::wind_at`), so a column leans and swings with the tufts
        /// under it. 0 rises straight up.
        smoke_wind_drift: f32 = 0.34 in 0.0 ..= 2.0;
        /// How fast a smoke puff grows, px/s. Rendered in whole 2px
        /// blocks, so this reads as a few discrete steps up rather than a
        /// smooth swell.
        smoke_growth: f32 = 4.0 in 0.0 ..= 100.0;
        smoke_opacity: f32 = 0.4 in 0.0 ..= 1.0;
        /// Particles a shot knocks off a tile it hits but does not kill.
        /// Well under `tile_burst_particles`: a wall being worn down should
        /// read as less than a wall coming apart.
        tile_chip_particles: i32 = 5 in 0 ..= 60;
        /// Particles a destroyed tile throws; glass and sandbag add to it.
        tile_burst_particles: i32 = 12 in 0 ..= 120;
        /// Sparks a dying tank throws.
        wreck_burst_particles: i32 = 16 in 0 ..= 200;
        /// Embers and smoke a burning wood tile gives off per second.
        wood_ember_rate: f32 = 20.0 in 0.0 ..= 200.0;
        wood_smoke_rate: f32 = 5.0 in 0.0 ..= 100.0;
        /// The sustained column off a wreck that is still burning, per
        /// second, for as long as `wreck_burn_seconds` lasts.
        wreck_flame_rate: f32 = 12.0 in 0.0 ..= 200.0;
        wreck_smoke_rate: f32 = 10.0 in 0.0 ..= 100.0;
        /// Smoke puffs a second off the engine deck of a hull on its last
        /// legs (`damage_stage::smoke`); a hull only just into the damaged
        /// tier gives off a sixth of it.
        hull_smoke_rate: f32 = 6.0 in 0.0 ..= 60.0;
        /// Contact feedback. `max_impulse` is the solver's own measure of
        /// how hard a contact is, so these are thresholds on that rather
        /// than on speed: below the first, a contact is a nudge and stays
        /// silent; at the second it is a real slam and throws sparks.
        contact_fx_min_impulse: f32 = 8.0 in 0.0 ..= 500.0;
        contact_fx_spark_impulse: f32 = 40.0 in 0.1 ..= 1000.0;
        /// Base emission rate while in contact, scaled by how hard it is.
        contact_fx_rate: f32 = 12.0 in 0.0 ..= 100.0;
        /// Dust kicked up per second while a tank grinds a prop under its
        /// tracks - the one part of `ram_props` that was visually silent.
        ram_dust_rate: f32 = 14.0 in 0.0 ..= 100.0;
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

    group view {
        /// The most the field is scaled up on screen (view.rs). The map is
        /// the same for every player in a match and always fully visible,
        /// so a big screen would otherwise blow it up - 2x on a 1080p
        /// monitor, a 35 mm tank - while a phone sees it at 8 mm; the cap
        /// draws it at this scale at most and fills the rest of the window
        /// with the backdrop's colour. 1.0 is the classic desktop look (a 64 px
        /// tank), 1.5 about the old window on a laptop. 0 turns the cap
        /// off. A phone is never affected: its fit is below any cap. Also
        /// `--zoom`. Live.
        view_max_scale: f32 = 1.5 in 0.0 ..= 8.0;
        /// Floor the cap to a half step (1.0, 1.5, 2.0 ...) so every art
        /// pixel is a whole number of screen pixels. Off: the pixel art may
        /// shimmer slightly at a fractional scale, which a static floor
        /// mostly hides.
        view_scale_snap: i32 = 0 in 0 ..= 1;
        /// How much world a field map shows on a screen, in 32 px cells
        /// (`framing`, docs/large-maps-follow-camera.md §3). 578 is the
        /// standard 34 x 17 field's area, the most a landscape phone shows
        /// at a readable tank. Every screen in a room shows this much world
        /// whatever its shape - the same amount of information and the
        /// same time to see a shell coming on a phone, a tablet and a
        /// monitor - and the screen picks only the outline. Arenas (36 x 18
        /// cells and smaller) are shown whole and never read it.
        view_area_cells: f32 = 578.0 in 64.0 ..= 4096.0;
        /// The narrowest outline a field map's view takes, as width over
        /// height: 4:3, a hair under so that a 4:3 screen is inside the
        /// range rather than a rounding error outside it. A narrower screen
        /// shows the 4:3 outline across its width and bars above and below.
        view_aspect_min: f32 = 1.3333 in 1.0 ..= 2.0;
        /// The widest outline a field map's view takes, as width over
        /// height: 2.4:1. A wider screen (a 32:9 monitor) shows the 2.4:1
        /// outline across its height and bars at the sides, where its HUD
        /// can sit; every shape between the two fills its screen.
        view_aspect_max: f32 = 2.4 in 1.5 ..= 4.0;
        /// A screen under this many pixels per inch snaps a field map's
        /// zoom to whole 2 px blocks - a multiple of 0.5 device pixels per
        /// world pixel, so the pixel art stays crisp - taking the nearer
        /// step by ratio, and the outward one wherever the nearer would
        /// hide the sight box. Tablets, laptops, monitors and the 720p
        /// phones snap. A finer screen (the flagship phones, over 400 ppi)
        /// keeps the exact zoom: its steps are about 20 % apart, either
        /// neighbour would push the sight box off screen or shrink the tank
        /// under 44 pt, and its pixels are too small for the blur to show.
        /// 0 keeps the exact zoom everywhere.
        view_fine_ppi: f32 = 360.0 in 0.0 ..= 1000.0;
        /// A local round - no room, alone or two on one screen - zooms a
        /// field map out a whole block at a time while a tank stays at
        /// least this wide, in millimetres, and the view holds at most
        /// `view_local_max_cells`: nobody shares the round, so a big
        /// screen may show more. A 24" or 27" monitor then shows 40 x 22.5
        /// cells with a 26.5 to 30 mm tank instead of the shared view's 30
        /// to 32 cells across at 35 to 40 mm. Phones, tablets and laptops
        /// draw a tank under 25 mm already and keep the shared view, a
        /// screen of unknown size keeps it too, and a room never zooms out.
        view_local_min_tank_mm: f32 = 25.0 in 0.0 ..= 100.0;
        /// The most world a local round's zoomed-out view shows, in cells:
        /// 900 is 40 x 22.5 on a 16:9 monitor. At or under
        /// `view_area_cells` no screen zooms out.
        view_local_max_cells: f32 = 900.0 in 64.0 ..= 4096.0;
    }

    group camera {
        /// The one motion switch (`motion.rs`, docs/large-maps-follow-camera.md
        /// section 6): 0 follows the platform - iOS's Reduce Motion, the
        /// browser's `prefers-reduced-motion`; Android, macOS, Linux and
        /// Windows say nothing, which is full motion -, 1 is full motion
        /// and 2 reduced. Reduced motion has no camera shake, no ripple
        /// bending the whole screen and no zoom at a round's opening (the
        /// establishing shot cuts to the tank); the follow camera, the
        /// arrows and every effect of the fight itself stay.
        reduce_motion: i32 = 0 in 0 ..= 2;
        /// The follow camera on a field map (`follow.rs`,
        /// docs/large-maps-follow-camera.md section 6): how far the seat
        /// moves inside the view on an axis, in pixels either way, before
        /// it drags the view along - about 0.4 cell, so four-way
        /// corrections and slides along a wall do not wobble the view.
        camera_dead_zone_px: f32 = 12.8 in 0.0 ..= 128.0;
        /// The look-ahead: the view leads the seat the way its hull faces,
        /// spending the room the sight box leaves on that axis
        /// (`Framing::room_outside`) less the dead zone - this fraction of
        /// it at rest, because a tank fires where it faces, and all of it
        /// at the tank's top speed. The sight box never leaves the screen
        /// whatever this says.
        camera_lead_at_rest: f32 = 0.35 in 0.0 ..= 1.0;
        /// Seconds the look-ahead takes to swing from one side of its room
        /// to the other: it moves at that steady pace, so a reversal takes
        /// about this long, a turn or a change of speed less, and a turn
        /// does not whip the view across. 0 swings it at once.
        camera_lead_ease_seconds: f32 = 0.5 in 0.0 ..= 5.0;
        /// Seconds a reversal holds before the look-ahead flips to the
        /// other side, so a quick back-and-forth does not swing the view
        /// each time; a turn to either side swings it at once.
        camera_lead_reverse_hold_seconds: f32 = 0.25 in 0.0 ..= 3.0;
        /// The spring the view chases its goal on: a critically damped
        /// spring of this smoothing time (Unity's `SmoothDamp`), the seat's
        /// velocity fed forward so a steady drive does not trail. 0 sticks
        /// the view to its goal.
        camera_spring_seconds: f32 = 0.5 in 0.0 ..= 2.0;
        /// Seconds a screen stays on its seat's wreck before it follows the
        /// nearest live teammate - in a wave round the seat comes back
        /// through a gate with the next wave, and the view cuts back to it
        /// then. Alone, the view stays on the wreck.
        camera_spectate_delay_seconds: f32 = 1.0 in 0.0 ..= 10.0;
        /// How far from the field's edge, in pixels, the view starts to
        /// ease into it: the goal it chases slows over this last stretch
        /// and never passes the edge, so the view comes to rest there
        /// rather than running into it and stopping dead. 0 is a hard
        /// stop.
        camera_edge_ease_px: f32 = 48.0 in 0.0 ..= 512.0;
        /// The establishing shot (`establish.rs`, docs/large-maps-follow-camera.md
        /// section 6): a field map's round opens on the whole map - the
        /// frog, the gates - for this many seconds, while its mission
        /// banner holds the round still. 0 plays no shot.
        camera_establish_hold_seconds: f32 = 1.0 in 0.0 ..= 5.0;
        /// ... then zooms down to the tank's follow view over this many
        /// seconds, or cuts to it under reduced motion. The shot always
        /// ends with the banner at the latest - a shorter banner shortens
        /// the hold first - and any steer or shot ends both at once, so it
        /// never costs a moment of play.
        camera_establish_zoom_seconds: f32 = 0.45 in 0.0 ..= 3.0;
    }

    group builder {
        /// The builder's own camera (`editor/camera.rs`,
        /// docs/large-maps-follow-camera.md section 9): how much one wheel
        /// notch or one `+`/`-` press zooms the canvas, as a ratio. On a
        /// coarse screen (under `view_fine_ppi`) the step lands on the
        /// nearest whole-block scale past it, so the 2 px blocks stay
        /// whole on the glass.
        builder_zoom_step: f32 = 1.25 in 1.05 ..= 2.0;
        /// The largest a cell is drawn when zoomed in, in points on the
        /// glass (128 is about 20 mm on a phone).
        builder_zoom_max_cell_pt: f32 = 128.0 in 32.0 ..= 512.0;
        /// How far a finger moves on the glass, in points, before a touch
        /// is a drag: a finger resting on the canvas paints nothing, and a
        /// two- or three-finger tap stays a tap.
        builder_touch_slop_pt: f32 = 10.0 in 0.0 ..= 48.0;
        /// How long a tap may last, in seconds: one finger paints a cell
        /// (or zooms in, `builder_paint_min_cell_mm`), two undo, three
        /// redo. A finger held longer without moving does nothing.
        builder_tap_seconds: f32 = 0.35 in 0.05 ..= 1.5;
        /// The paint threshold: where a cell is drawn smaller than this on
        /// the glass, in millimetres, a finger cannot hit one cell, so a
        /// one-finger tap zooms in (`builder_tap_zoom_cell_mm`) and a
        /// drag pans instead of painting. Touch only - a mouse paints at
        /// any size. 0 always paints.
        builder_paint_min_cell_mm: f32 = 6.0 in 0.0 ..= 20.0;
        /// The cell a zooming tap brings the canvas to, in millimetres on
        /// the glass, about the tapped point.
        builder_tap_zoom_cell_mm: f32 = 9.0 in 3.0 ..= 30.0;
        /// The loupe (docs/large-maps-patterns.md, "Touch editing without
        /// clashes, and a loupe"): while one finger paints a stroke where
        /// a cell is drawn smaller than this on the glass, in millimetres
        /// - a fingertip's width and some, so the finger hides the cell it
        /// is on - a magnified view of the cells under the finger stands
        /// above it, the cell the stroke paints outlined. Above the paint
        /// threshold (`builder_paint_min_cell_mm`), under which a finger
        /// paints nothing, and above the zoom a tap brings
        /// (`builder_tap_zoom_cell_mm`, which a coarse screen rounds up a
        /// little onto whole blocks), so a stroke after a zooming tap
        /// still has it. Touch only; 0 never shows it.
        builder_loupe_cell_mm: f32 = 12.0 in 0.0 ..= 30.0;
        /// The loupe's magnification over the canvas, before it is put on
        /// the nearest whole-block scale (`MapEditor::loupe`): the cell a
        /// finger paints and part of each of its neighbours, larger than
        /// the finger leaves them.
        builder_loupe_zoom: f32 = 1.5 in 1.0 ..= 4.0;
        /// How far the loupe stands off the point under a painting finger,
        /// in points: above it, clear of the fingertip - or beside it near
        /// the canvas's right edge and where there is no room above.
        builder_loupe_lift_pt: f32 = 44.0 in 0.0 ..= 160.0;
        /// Edge scroll: a stroke whose pointer comes within this many
        /// points of the canvas's edge scrolls the view toward that edge
        /// while it is held, so a long wall needs no pan in the middle.
        /// 0 turns it off.
        builder_edge_scroll_pt: f32 = 40.0 in 0.0 ..= 160.0;
        /// How fast edge scroll moves the view at the very edge, in points
        /// per second; it ramps up from nothing across the margin.
        builder_edge_scroll_pt_per_s: f32 = 600.0 in 0.0 ..= 4000.0;
        /// How fast a held arrow key pans the canvas, in points per second.
        builder_key_pan_pt_per_s: f32 = 800.0 in 0.0 ..= 4000.0;
        /// What the CHECK panel's jump to a finding shows round its cells
        /// at least, in cells across (`MapEditor::frame_cells`): a one-cell
        /// finding is seen in its surroundings rather than filling the
        /// canvas.
        builder_lint_jump_cols: f32 = 14.0 in 2.0 ..= 64.0;
        /// What the CHECK panel's jump to a finding shows round its cells
        /// at least, in cells down (`builder_lint_jump_cols` across).
        builder_lint_jump_rows: f32 = 9.0 in 2.0 ..= 64.0;
        /// The most cells one FILL changes (`editor::brush::flood`): a
        /// fill that would take more is refused, the status line saying
        /// so, rather than flooding a 250 x 250 map in one frame when it
        /// finds its way out through a gap. RECT is not held to it - its
        /// rectangle is the one drawn. 4096 cells of water or wall cost
        /// about 10 ms with their repaint in a release build
        /// (`a_large_fill_timing`).
        builder_fill_max_cells: usize = 4096 in 16 ..= 62500;
        /// SCATTER's footprint: the cells within this many cells of each
        /// cell the stroke crosses (and a half), a disc twice as wide and
        /// one more - 2 is 21 cells, five across.
        builder_scatter_radius_cells: i32 = 2 in 0 ..= 8;
        /// The share of SCATTER's footprint a stroke paints, chosen by a
        /// hash of the cell and the stroke rather than any random draw;
        /// another stroke over the same ground picks other cells, so going
        /// over it again thickens the scatter.
        builder_scatter_density: f32 = 0.25 in 0.02 ..= 1.0;
    }

    group online {
        /// How far behind the server an online round is drawn, in
        /// milliseconds, at least (docs/online-coop-prd.md §4.5, §4.16,
        /// `net::interp`) - the *floor* of the delay. A room sends sixty
        /// snapshots a second; drawing this far in the past keeps two
        /// snapshots bracketing render time, so the picture moves every
        /// frame rather than running on guesses. Below one snapshot
        /// interval (16.7 ms at the room's 60 Hz) the replica extrapolates
        /// most frames, which is the least this can sensibly take.
        ///
        /// With `online_interpolation_adaptive` on - the default - the
        /// delay the picture steers for is never below one interval plus
        /// one 60 Hz frame, 33.3 ms, so this floor binds only below that
        /// and only matters with the adaptive delay off, where it is the
        /// delay itself: lower it to trade smoothness for freshness, raise
        /// it on a jittery link. It is the largest single term in the
        /// latency budget (section 5) that is a choice rather than a cost,
        /// paid by the tanks a player aims *at* and everything not
        /// predicted. Live, so settings can be compared mid-round
        /// (`--rig --delay 80 --jitter 20`).
        online_interpolation_delay_ms: f32 = 33.0 in 0.0 ..= 500.0;
        /// Size the interpolation delay from the link's own lateness
        /// (docs/online-coop-prd.md §4.5, §4.16, decision 8;
        /// `net::interp`). On, the delay the picture steers for is one
        /// measured snapshot interval (the bracket's width) plus one
        /// 60 Hz frame plus the 95th percentile, over the last three
        /// seconds, of how late snapshots arrive behind the fastest - with
        /// head-of-line stalls left out while they are isolated, since
        /// those are ridden out on extrapolation rather than paid for on
        /// every frame - floored at `online_interpolation_delay_ms` and
        /// capped at `online_interpolation_delay_max_ms`. The picture
        /// reaches it through its own playout clock, a few per cent
        /// faster or slower than real time and never backwards, so a
        /// jittery spell never rewinds it. Off pins the delay at the
        /// floor, which is how the two are judged side by side on one
        /// link (`--rig --jitter 20`). Live.
        online_interpolation_adaptive: bool = true in 0 ..= 1;
        /// The most the adaptive delay may reach, in milliseconds. Past
        /// this a link is not worth smoothing over: the picture would be
        /// a fifth of a second behind and the shots plainly late, and the
        /// lateness above it is ridden out on extrapolation instead.
        /// Live.
        online_interpolation_delay_max_ms: f32 = 200.0 in 0.0 ..= 500.0;
        /// Run the local seat's own hull ahead of the server and
        /// reconcile it against each snapshot (stage 2,
        /// docs/online-coop-prd.md section 4.12, `net::predict`).
        ///
        /// Off, your own tank answers the stick
        /// `online_interpolation_delay_ms` plus half a round trip later,
        /// like every other hull. On, it answers on the next frame and
        /// the server's answer arrives as a correction - eased off over
        /// `NUDGE_SECONDS`, or taken whole past `SNAP_PX`.
        ///
        /// A knob rather than a constant because the two have to be
        /// judged side by side on the same link, which is what a rig run
        /// is for (`--rig --delay 120 --jitter 20`). Live: it takes
        /// effect on the next frame, and turning it off hands the drawn
        /// hull straight back to the interpolator and draws no
        /// provisional shots. A client that owns its hull
        /// (`online_client_hull`) keeps its sandbox in step with the room
        /// either way - the room's world, shoves and `Fired` - since every
        /// pose it sends comes from there; only the drawing follows this.
        online_predict_own_tank: bool = true in 0 ..= 1;
        /// Also draw this seat's shots on the frame of the press
        /// (docs/online-coop-prd.md sections 4.12 and 4.16,
        /// `net::predict`): each is the only drawn copy of its shot for
        /// its whole life, runs its projectile's own state machine, and
        /// stops at the first tile, tank or frog it meets in the drawn
        /// world with its impact drawn at once, the room's copy kept off
        /// the picture while it stands for it. A laser's beam and each
        /// shot's muzzle ripple are drawn on the press too.
        ///
        /// The room judges the shot against the hulls this client was
        /// drawing when it pressed (lag compensation, section 4.16), so a
        /// shot stopped at a drawn hull is the room's hit too; where the
        /// room still disagrees its copy is shown from there on and the
        /// disagreement counted (`crossings_hit`/`crossings_missed` on
        /// `status.round.prediction`). Live: off, the shot is the room's
        /// to draw, a round trip and the picture's delay after the press.
        /// Drawn only with `online_predict_own_tank` on; the hull's own
        /// prediction is unaffected either way.
        online_predict_shots: bool = true in 0 ..= 1;
        /// How much of the authored wave each seat past the first adds to a
        /// room's round (docs/online-coop-prd.md section 4.11): the room
        /// sends `wave_size_scale = 1 + (seats - 1) * this` in its
        /// `Welcome`, so at 1.0 a team of four meets four times the wave
        /// one player does and at 0.0 nothing scales at all. Under 1.0 on
        /// purpose - a team shares a field, focuses fire and covers each
        /// other, so it is worth more than the sum of its tanks; the number
        /// is the probe's `--players` sweeps flattened against the solo
        /// round (docs/maps-to-levels.md "Difficulty by seat count").
        ///
        /// **0.5, down from 0.75.** At 0.75 a room of two met 1.75x the
        /// authored wave, which turned the default map's opening into
        /// seven tanks before a shot was fired - the scaling multiplies
        /// the *first* wave, not just the ramp, so it is felt hardest
        /// where a round is judged. Half a wave per seat keeps a duo's
        /// opening one tank above the solo round and still has a team of
        /// eight meeting four and a half times the wave.
        ///
        /// Read by the room server alone, when a round starts. Turning it
        /// down makes every room easier; a local round never reads it.
        online_wave_size_per_seat: f32 = 0.5 in 0.0 ..= 2.0;
        /// A room's wave plan climbs one rung of the tier ladder
        /// (`wave_tier_step`) for every this many seats past the first, so
        /// a big team meets heavier chassis and not only more of them. At
        /// 3, two and three seats fight the authored tiers, four fight one
        /// rung up and eight two, and a ramp already at the top of the
        /// ladder simply stays there. The coarse dial of the two -
        /// `online_wave_size_per_seat` is the fine one.
        online_wave_tier_seats_per_step: usize = 3 in 1 ..= 8;
        /// The client owns its own hull in an online round
        /// (docs/online-coop-prd.md section 4.14, stage 3): every tick it
        /// sends where its tank is and the room puts the seat there,
        /// validated against the chassis's speed, the walls and deep
        /// water, instead of driving it from the stick and having the
        /// client predict and reconcile. On, the own hull never takes a
        /// correction and the shot leaves from where it was drawn; off,
        /// stage 2's prediction runs. Restart: read when a round is
        /// opened, so the two are compared round by round on one link.
        online_client_hull: bool = true in 0 ..= 1 @ Restart;
    }

    group cosmetics {
        // --- portals (portal.rs) ---
        /// Seconds for one visual revolution of a portal's spiral - the
        /// twelve baked frames cycle once over this. Slow on purpose: a
        /// black hole turns, it does not spin.
        portal_spin_seconds: f32 = 4.0 in 0.5 ..= 30.0;
        /// Peak opacity of the additive blue glow under an active portal
        /// (0 turns it off).
        portal_glow_strength: f32 = 0.35 in 0.0 ..= 1.0;
        // --- tall grass (grass.rs) ---
        /// Tufts scattered per tall-grass cell.
        ///
        /// Several rather than one because the occlusion has to be
        /// *partial* - the reference gif never hides a unit completely,
        /// it walks between discrete tufts, covering at worst ~44% of it.
        /// This knob and the tuft height in `gen_grass.py` are the same
        /// trade-off from two directions, so both were measured against a
        /// parked tank rather than guessed:
        ///
        /// | tufts/cell | tank occluded |
        /// |---|---|
        /// | 2 | 56% |
        /// | 3 | 71% |
        /// | 4 | 83% |
        ///
        /// 2 sits closest to the reference while still reading as a field.
        /// Above 3 a tank vanishes outright, which breaks the player's
        /// ability to find their own hull - see docs/GROUND_SPEC.md.
        grass_tufts_per_cell: i32 = 2 in 0 ..= 24 @ Restart;
        /// On-screen scale of the 32px tuft cells. 2.0 puts one *sheet*
        /// pixel on a 2x2 screen block, the density everything else uses -
        /// which is why `gen_grass.py` authors one design pixel per sheet
        /// pixel rather than on the walls sheet's 2px block grid. Stacking
        /// the two made grass twice as chunky as the world around it.
        grass_scale: f32 = 2.0 in 0.5 ..= 4.0;
        /// How far a tuft may stand in front of the foot of a wall to its
        /// north, px over the wall's bottom edge (`grass::keep_off`): a
        /// quarter of a cell, about one course of bricks, reads as grass
        /// growing at the wall's foot; more covers the wall itself. A wall
        /// beside or below a tuft is never overlapped.
        grass_wall_overlap_px: f32 = 8.0 in 0.0 ..= 16.0 @ Restart;
        /// Each tuft's own flutter on top of the wind (`grass::bend`): how
        /// far a tip travels, and how fast. Hashed per tuft, so it is what
        /// keeps a field bending in a gust from moving as one sheet.
        grass_sway_px: f32 = 0.6 in 0.0 ..= 20.0;
        grass_sway_speed: f32 = 1.6 in 0.0 ..= 20.0;
        /// The steady lean of every tuft along the wind, in px at the tip
        /// (`grass::wind_at`). A tuft bends about its root, so only the
        /// wind's sideways component shows.
        grass_wind_px: f32 = 1.2 in 0.0 ..= 10.0;
        /// How much further a gust bends the grass, px at the tip. Gusts are
        /// fronts that roll across the field, so neighbouring tufts bend
        /// together and a gust is seen crossing the meadow.
        grass_gust_px: f32 = 3.6 in 0.0 ..= 20.0;
        /// Which way the gust fronts travel, degrees clockwise from the +x
        /// axis on the y-down screen. 0 blows left to right; 180 flips the
        /// lean too.
        grass_gust_heading_deg: f32 = 22.0 in -180.0 ..= 180.0;
        /// How fast the fronts travel across the field, px per second.
        grass_gust_speed: f32 = 130.0 in 0.0 ..= 1000.0;
        /// Px between one wave of bending grass and the next.
        grass_gust_wavelength: f32 = 360.0 in 16.0 ..= 4000.0;
        /// Px between the peaks of the slower envelope the waves ride on:
        /// the size of one gust. Longer than the wavelength, so a gust is a
        /// few waves strong and then the field settles to its steady lean.
        grass_gust_group: f32 = 1600.0 in 16.0 ..= 10000.0;
        /// The wake: how close a tank has to be to shove grass aside, and
        /// how hard at the centre. Grass *ahead* of a moving tank is pushed
        /// harder than grass behind it, so a hull drives a bow wave rather
        /// than a symmetric ring (`grass::tick`). Not in the reference gif -
        /// grass there does not react at all - so this is the first thing to
        /// turn down if it reads as noisy.
        grass_part_radius: f32 = 46.0 in 0.0 ..= 300.0;
        grass_part_px: f32 = 9.0 in 0.0 ..= 60.0;
        /// How far *past the hull* a tank flattens grass. Measured from the
        /// hull box, not from the tank's centre - a radial falloff from the
        /// centre leaves the grass under the tracks standing, because the
        /// hull is wider than any radius small enough to look right. So
        /// this is the margin around the footprint, and
        /// `grass_part_radius` is the wider ring that only bends.
        grass_crush_radius: f32 = 16.0 in 0.0 ..= 200.0;
        /// How long flattened grass takes to stand back up. This is the
        /// whole trail effect - a tank leaves a matted path that closes
        /// behind it, the same shape `track.rs` gives a tread mark. Short
        /// values read as grass springing back instantly and lose the path.
        grass_crush_recover_seconds: f32 = 3.5 in 0.1 ..= 30.0;
        /// How far a fully flattened tuft is squashed toward its own root.
        /// Never 1.0: a tuft that disappears entirely reads as a hole in
        /// the field rather than as matted grass.
        grass_crush_flatten: f32 = 0.78 in 0.0 ..= 0.95;
        /// Leaf or straw specks a tank kicks up per second per grass cell it is
        /// crossing (`fx.rs`, scaled by `fx_density` like every other
        /// emitter). Zero turns the rustle off.
        grass_rustle_rate: f32 = 14.0 in 0.0 ..= 120.0;

        // --- the ground layer's baked floor shade (ground::bake_shade) ---
        /// The walls' contact shade: the opacity of its darkest step, 0-1,
        /// toward a cool dark (`ground::WALL_SHADE_COLOR`). Walls stand
        /// *on* the floor, and without this they read as pasted onto it.
        /// Stepped and dithered on the 2 px grid, per block, so it follows
        /// a wall's outline rather than the 32 px cell grid.
        ground_wall_shade: f32 = 0.17 in 0.0 ..= 1.0 @ Restart;
        /// How far that shade reaches from a wall, in cells.
        ground_wall_shade_cells: f32 = 1.4 in 0.0 ..= 8.0 @ Restart;
        /// How far the walls' shade leans along the drop shadows' direction
        /// (`shadow_dir_x`/`_y`), px: it pools on the side the walls' own
        /// shadows fall and thins on the lit side.
        ground_wall_shade_lean_px: f32 = 10.0 in 0.0 ..= 48.0 @ Restart;
        /// The edge shade: the opacity of its darkest step, 0-1, toward the
        /// theme's darkest ground (canopy green, dusk umber) rather than
        /// black. A round's field only - the builder's canvas has none.
        ground_edge_shade: f32 = 0.30 in 0.0 ..= 1.0 @ Restart;
        /// How far the edge shade reaches in from the middle of an edge,
        /// in screen px.
        ground_edge_shade_px: f32 = 110.0 in 0.0 ..= 600.0 @ Restart;
        /// How round the edge shade's corners are, as a multiple of its
        /// reach: 0 is a square frame, 1 rounds each corner by the reach.
        ground_edge_shade_round: f32 = 1.0 in 0.0 ..= 4.0 @ Restart;
        /// The share of the edge shade's darkest step only the corners
        /// reach, 0-1: the middle of an edge stops short of it.
        ground_edge_shade_corner: f32 = 0.25 in 0.0 ..= 1.0 @ Restart;
        /// The ground past an arena's field, in the window's margins
        /// (`margin.rs`): the opacity the edge shade deepens to out there,
        /// 0-1, toward the same dark - the shade the world off the
        /// playfield stands in. Taken in whole steps of the edge shade's.
        ground_margin_shade: f32 = 0.6 in 0.0 ..= 1.0 @ Restart;
        /// How far past the field's edge, in world px, the edge shade
        /// deepens to `ground_margin_shade`.
        ground_margin_ramp_px: f32 = 48.0 in 0.0 ..= 600.0 @ Restart;
        /// How much of the open floor the soft sand patches cover, 0-1:
        /// the pack's sand tiles (hardpan under the desert retint) laid
        /// where a hashed value noise at the cell corners crosses this
        /// coverage, and resolved through the pack's own corner autotile
        /// so every edge is hand-painted (`ground::SAND_CORNER`). Never
        /// beside a road cell, and only on a theme that asks for them
        /// (`map::Theme::drifts` - the desert); 0 turns them off there too.
        ground_drift_cover: f32 = 0.32 in 0.0 ..= 1.0 @ Restart;
        /// The patches' scale: the noise lattice pitch in cells. Larger
        /// means fewer, bigger drifts.
        ground_drift_scale: f32 = 5.0 in 1.0 ..= 20.0 @ Restart;
        /// Seconds each of the pack's four water frames stays up
        /// (`ground::WATER_FRAMES`): the shimmer on shores and streams.
        /// The pack's own timing is 0.1.
        water_frame_seconds: f32 = 0.14 in 0.02 ..= 1.0;
        /// How fast the current's marks drift down the map over open lake
        /// water and along north/south streams, world px per second
        /// (`ground::draw_current`). Zero holds them still.
        water_flow_speed: f32 = 18.0 in 0.0 ..= 200.0;
        /// Marks per column of water. Zero turns the current off and
        /// leaves the pack's shimmer.
        water_flow_lanes: i32 = 2 in 0 ..= 6;

        // --- what water does to a hull (docs/water.md) ---
        /// A ford's pace: the fraction of its top speed and of
        /// `tank_accel_force` a hull in shallow water gets, so it wades
        /// rather than drives and a hull entering at speed is pulled down
        /// to the wading pace. Deep water is not scaled, it is a wall.
        water_speed_factor: f32 = 0.55 in 0.05 ..= 1.0;
        /// A ford's grip: the fraction of `tank_turn_grip_force` a hull in
        /// shallow water keeps, so a turn sloshes wide and momentum
        /// carries it.
        water_grip_factor: f32 = 0.5 in 0.0 ..= 1.0;
        /// The current: how fast the water in a stream joined north or
        /// south moves down the map, px/s. A hull in it drives relative to
        /// the water (`drive_tank_with`), so one that stops drifts south at
        /// this speed and one crossing has to aim upstream. Zero turns it
        /// off.
        water_current_speed: f32 = 28.0 in 0.0 ..= 200.0;
        /// What a step into a ford costs the AI's router, in dry steps
        /// (`pathfind::Grid::weigh`): 1 makes water free, more sends a tank
        /// round a river whenever the detour is shorter than the extra
        /// this charges. Deep water is blocked outright.
        water_ford_path_cost: i32 = 3 in 1 ..= 20 @ Restart;
        /// Seconds a hull leaves wet tread marks after wading out. The
        /// marks are darker (`water_wet_track_darken` times the dry
        /// opacity) and fade over this same time.
        water_wet_track_seconds: f32 = 2.5 in 0.0 ..= 20.0;
        /// How much darker a wet tread mark is than a dry one.
        water_wet_track_darken: f32 = 1.7 in 1.0 ..= 3.0;
        /// Droplets a wading hull throws per second at full speed (fx.rs,
        /// scaled by `fx_density`); zero turns the spray off.
        water_spray_rate: f32 = 45.0 in 0.0 ..= 300.0;
        /// World px of travel between hull tread-animation frame advances
        /// (independent of the ground-decal spacing below).
        tank_hull_track_frame_distance: f32 = 8.0 in 1.0 ..= 64.0;
        /// Seconds a turret holds each recoil cell after its main gun fires
        /// (`Tank::kick`): the barrel kicked back, then a twin's second
        /// barrel or a single one on its way home. Presentation only.
        tank_recoil_seconds: f32 = 0.08 in 0.0 ..= 0.5;
        /// Drop shadows: shared screen-space offset direction (down-right, a
        /// top-down-arcade convention) - only the distance differs per
        /// entity type. See docs/sprite-shadows-design.md.
        shadow_dir_x: f32 = 0.595 in -1.0 ..= 1.0;
        shadow_dir_y: f32 = 0.48 in -1.0 ..= 1.0;
        /// Tank shadow distance (px) - grounded, stays tight to the hull.
        tank_shadow_offset: f32 = 3.0 in 0.0 ..= 20.0;
        tank_shadow_opacity: f32 = 0.486 in 0.0 ..= 1.0;
        /// Ground ring radius as a multiple of `Tank::size()`: the marker,
        /// health and shield rings all share it. Drawn under the tank and
        /// its shadow, so only what reaches past the hull shows.
        shield_glow_radius_factor: f32 = 0.385 in 0.1 ..= 2.0;
        /// How many full rainbow hue cycles the shield ring makes per second.
        shield_glow_hue_hz: f32 = 0.4 in 0.0 ..= 5.0;
        /// The shield ring fades out below this fraction of charge, so the
        /// wearer can see it about to shatter and the health ring underneath
        /// cross-fades back in. A fraction rather than a countdown because
        /// the shield has no clock - what is running out is absorption.
        shield_glow_fade_fraction: f32 = 0.15 in 0.0 ..= 1.0;
        /// Opacity of the player's white ground ring: high enough to read
        /// as white rather than grass-tinted grey, with a little ground
        /// showing through. The shield ring's own translucency is fixed in
        /// `draw_ground_ring`.
        player_ring_opacity: f32 = 0.8 in 0.0 ..= 1.0;
        /// Every tank ring (marker, health gauge and shield, player and
        /// enemy alike) is this much larger in radius than the
        /// `shield_glow_radius_factor` ring the frogs use, at the same band
        /// thickness, so a halo shows past every hull - the super-heavies
        /// hide the unscaled ring entirely.
        tank_ring_radius_scale: f32 = 1.1 in 0.5 ..= 2.0;
        /// For this long after a round becomes playable (after the mission
        /// banner, or at once without one) each player tank pulses a
        /// team-coloured ripple ring and shows its `P1`/`P2` label, so you
        /// find yourself in a crowd. 0 disables the cue.
        player_locate_seconds: f32 = 2.0 in 0.0 ..= 10.0;
        /// Pulses per second of the locate ripple.
        player_locate_pulse_hz: f32 = 2.0 in 0.2 ..= 8.0;
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
        /// Enemy health ring: after a hit an enemy's ground ring shows its
        /// health for this many seconds ...
        health_ring_hit_seconds: f32 = 3.0 in 0.0 ..= 20.0;
        /// ... fading out over the trailing this-many seconds of that window.
        health_ring_hit_fade_seconds: f32 = 0.6 in 0.0 ..= 5.0;
        /// An enemy's health ring stays on, hit or not, once its remaining
        /// health is at or below this fraction.
        enemy_health_ring_below: f32 = 0.5 in 0.0 ..= 1.0;
        /// The missing part of a white or red health ring (the player's
        /// tank, both frogs) is still drawn, at `player_ring_opacity` times
        /// this, so the marker stays a full circle.
        health_ring_base_opacity: f32 = 0.35 in 0.0 ..= 1.0;
        /// Opacity of the dark band over the missing part of an enemy
        /// tank's health ring.
        health_ring_gap_opacity: f32 = 0.45 in 0.0 ..= 1.0;
        /// HUD numbers (SHELLS/HP) turn orange below this fraction of max
        /// ...
        hud_warn_threshold: f32 = 0.34 in 0.0 ..= 1.0;
        /// ... and red below this. Conservative on purpose: only flag real
        /// trouble.
        hud_critical_threshold: f32 = 0.104 in 0.0 ..= 1.0;
    }

    group indicators {
        /// How far inside the screen's safe area the off-screen arrows sit
        /// (points): the rectangle the line from the tank to what an arrow
        /// points at stops on (`indicators.rs`,
        /// docs/large-maps-follow-camera.md section 7).
        indicator_inset_pt: f32 = 10.0 in 0.0 ..= 64.0;
        /// An arrow's size at full scale (points). An arrow slid clear of
        /// a HUD cluster, a thumb's rest or the minimap stops half this
        /// short of it.
        indicator_arrow_pt: f32 = 16.0 in 4.0 ..= 64.0;
        /// Distance is drawn as size and opacity, counted in screens (the
        /// view's extent along the arrow): full size and opacity up to
        /// this many screens away ...
        indicator_near_screens: f32 = 1.0 in 0.0 ..= 10.0;
        /// ... shrinking to `indicator_far_scale` of the size and
        /// `indicator_far_alpha` of the opacity at this many, and no
        /// further beyond.
        indicator_far_screens: f32 = 4.0 in 0.5 ..= 20.0;
        indicator_far_scale: f32 = 0.6 in 0.1 ..= 1.0;
        indicator_far_alpha: f32 = 0.55 in 0.0 ..= 1.0;
        /// Enemy arrows that land closer together than this (points) merge
        /// into one that carries a count. Teammates, frogs and gates never
        /// merge.
        indicator_cluster_pt: f32 = 22.0 in 0.0 ..= 120.0;
        /// The most arrows a screen shows at once, filled by priority: lane
        /// threats, teammates, frogs, flashing gates, then the nearest
        /// enemies, the enemies left over folded into one count per screen
        /// edge. Teammates and frogs always show, whatever this says.
        indicator_max_arrows: usize = 8 in 1 ..= 32;
        /// An enemy in tall grass gets no arrow unless it fired within
        /// this many seconds ...
        indicator_reveal_fire_seconds: f32 = 1.5 in 0.0 ..= 10.0;
        /// ... or stands within this many pixels of the seat (two cells).
        indicator_reveal_px: f32 = 64.0 in 0.0 ..= 512.0;
        /// Seconds the hollow marker an enemy leaves where it slipped out
        /// of sight lasts, fading. It never moves.
        indicator_last_seen_seconds: f32 = 4.0 in 0.0 ..= 30.0;
        /// Seconds the arrow of an enemy lined up on the seat flashes after
        /// it fires down the lane.
        indicator_fire_flash_seconds: f32 = 0.3 in 0.0 ..= 2.0;
        /// Seconds the arc on the seat's own tank points back the way the
        /// last hit came.
        indicator_hit_arc_seconds: f32 = 0.8 in 0.0 ..= 5.0;
        /// Seconds a wave gate flashes after a tank starts rolling in
        /// through it, and again after one comes through.
        indicator_gate_flash_seconds: f32 = 3.0 in 0.0 ..= 20.0;
        /// How fast the ring round a lined-up enemy's arrow pulses, per
        /// second; it brightens and the arrow swells as the enemy's aim
        /// settles (`indicators::picture`).
        indicator_pulse_hz: f32 = 5.0 in 0.5 ..= 20.0;
        /// How much bigger a lined-up enemy's arrow is at the top of its
        /// pulse once the aim has settled: 0.2 is a fifth.
        indicator_pulse_swell: f32 = 0.2 in 0.0 ..= 1.0;
        /// How fast a flashing gate's marks blink, on and off per second,
        /// as does a teammate's arrow while it drives back in.
        indicator_gate_blink_hz: f32 = 3.0 in 0.5 ..= 20.0;
        /// The hit arc's inner radius round the seat's tank, in world
        /// pixels: just outside the ground ring.
        indicator_hit_arc_px: f32 = 32.0 in 8.0 ..= 128.0;
        /// How far round the tank the hit arc reaches, in degrees, centred
        /// on the way the hit came.
        indicator_hit_arc_degrees: f32 = 70.0 in 10.0 ..= 360.0;
        /// Where a touch screen's thumbs rest, which no arrow sits under:
        /// a pad this many millimetres wide ...
        indicator_thumb_pad_mm: f32 = 12.0 in 0.0 ..= 60.0;
        /// ... centred this far in from each side of the screen ...
        indicator_thumb_in_mm: f32 = 22.0 in 0.0 ..= 80.0;
        /// ... and this far up from its bottom edge.
        indicator_thumb_up_mm: f32 = 18.0 in 0.0 ..= 80.0;
    }

    group ui {
        /// How large the chrome is drawn - the corner clusters, the
        /// banners, the dialogs, the end screen, the lobby and the level
        /// select - in points: at 1 a 12 pt label is 12 points tall on a
        /// phone, a tablet and a monitor alike, whatever scale the world is
        /// drawn at (`hud::UiFrame`, docs/large-maps-follow-camera.md
        /// section 8). A window too small for the chrome at this size draws
        /// it smaller, to fit.
        ui_scale: f32 = 1.0 in 0.5 ..= 3.0;
        /// A corner cluster drops to this opacity while a tank, a shot or
        /// a blast is under it, so the HUD never hides the fight ...
        ui_fade_opacity: f32 = 0.35 in 0.0 ..= 1.0;
        /// ... moving there, and back once the fight has passed, over this
        /// many seconds. 0 snaps.
        ui_fade_seconds: f32 = 0.25 in 0.0 ..= 2.0;
        /// In a build with the dev tools, where each frame's time goes -
        /// the frame, the steps, the pictures kept, the lights, the world,
        /// the chrome, the swap (`frame_stages.rs`) - on a line under the
        /// left cluster: a PR preview's tuning panel turns it on, on a
        /// phone too. Nothing in any other build.
        ui_frame_stages: bool = false in 0 ..= 1;
    }

    group minimap {
        /// When the play minimap is drawn under the top-right cluster
        /// (`minimap.rs`, docs/large-maps-follow-camera.md sections 7 and
        /// 15), and only ever while the camera shows less than the whole
        /// field: 0 never, 1 on tablets and desktops but not on a phone
        /// (`minimap_phone_short_pt`), whose edge arrows carry the field,
        /// 2 on every screen. The builder's navigator is on every device
        /// whatever this says.
        minimap_show: i32 = 1 in 0 ..= 2;
        /// A screen whose short side is under this many points is a
        /// phone's: a landscape phone is 360 to 440 points tall, the
        /// smallest tablet 744 ...
        minimap_phone_short_pt: f32 = 500.0 in 0.0 ..= 2000.0;
        /// ... and so is one under this many millimetres, where the
        /// platform reports the screen's size: a small panel that reports a
        /// desktop's points.
        minimap_phone_short_mm: f32 = 90.0 in 0.0 ..= 1000.0;
        /// The box the minimap is fitted into, in points, the map's shape
        /// kept: a long map as wide as this and shorter ...
        minimap_width_pt: f32 = 160.0 in 48.0 ..= 480.0;
        /// ... a tall one as tall as this and narrower. The builder's
        /// navigator is fitted into the same box.
        minimap_height_pt: f32 = 120.0 in 32.0 ..= 480.0;
    }

    group fx {
        /// One multiplier on every effect that touches the whole screen -
        /// the kill flash, the shockwave ripple's bend, the camera shake
        /// and a storm's lightning - so a calmer or reduced-flash mode is
        /// one slider. 0
        /// leaves only the local fireball, glow, impact quad and
        /// particles, which deliberately stay out of it.
        screen_fx_intensity: f32 = 1.0 in 0.0 ..= 2.0;
        /// Kill shockwave (shockwave.fs): seconds the effect plays before
        /// clearing.
        shockwave_duration: f32 = 0.7 in 0.05 ..= 5.0;
        /// Ring growth speed, in the standard field's heights a second
        /// (`shockwave::RIPPLE_FRAME`, 544 px each) - the same in world
        /// pixels on every map.
        shockwave_speed: f32 = 0.56 in 0.0 ..= 5.0;
        /// Thickness of the distorted band, in standard field heights.
        shockwave_width: f32 = 0.08 in 0.0 ..= 1.0;
        /// How hard the ring bends the image, in standard field heights.
        shockwave_strength: f32 = 0.045 in 0.0 ..= 0.5;
        /// Camera shake on the same kill trigger: duration (much shorter
        /// than the shockwave so it reads as one punchy hit), px offset at
        /// full strength, and radians/sec of the wobble.
        camera_shake_duration: f32 = 0.22 in 0.0 ..= 3.0;
        /// Ceiling on the summed camera shake when several ripples overlap,
        /// as a multiple of `camera_shake_magnitude`. Without it three
        /// simultaneous kills throw the scene far enough that the screen
        /// edge shows through as black.
        camera_shake_max_stack: f32 = 1.5 in 1.0 ..= 5.0;
        camera_shake_magnitude: f32 = 6.0 in 0.0 ..= 100.0;
        camera_shake_frequency: f32 = 40.0 in 1.0 ..= 200.0;
        /// The shake falls off with distance from what the screen shows
        /// (`shockwave::camera_shake`, docs/large-maps-follow-camera.md
        /// section 6), so a blast across a field map does not shake a
        /// screen that cannot see it: a ripple within this many pixels of
        /// the view shakes it fully ...
        camera_shake_margin_px: f32 = 160.0 in 0.0 ..= 2000.0;
        /// ... and one this many screens past that - a screen being the
        /// view's width across and its height up and down - not at all,
        /// fading between: at 1, a blast a screen and a half from the
        /// view's middle is gone, as a blast on the far shore of a field
        /// map is. An arena's view is the whole field, which every ripple
        /// is in, so an arena's shake is every ripple's whole. 0 stops the
        /// shake at the margin.
        camera_shake_fade_screens: f32 = 1.0 in 0.0 ..= 20.0;
        /// Muzzle-flash heat haze (muzzle_flash.fs): a one-sided outward
        /// puff at the barrel. Hits full strength at the leading edge, so
        /// tuned lower than the shockwave for similar visual intensity.
        muzzle_flash_duration: f32 = 0.12 in 0.01 ..= 2.0;
        /// Front growth, in standard field heights a second
        /// (`shockwave::RIPPLE_FRAME`).
        muzzle_flash_speed: f32 = 0.9 in 0.0 ..= 5.0;
        /// Thickness of the pushed band, in standard field heights.
        muzzle_flash_width: f32 = 0.032 in 0.0 ..= 0.5;
        /// How hard the puff shoves the image, in standard field heights.
        muzzle_flash_strength: f32 = 0.015 in 0.0 ..= 0.5;
        /// Half-extent (world px) of the quad the muzzle flash is drawn
        /// into - it must hold the puff's whole reach, speed x duration
        /// plus the band in standard field heights of 544 px (about 76 px
        /// at the defaults), or the puff visibly clips.
        muzzle_flash_quad_radius: f32 = 90.0 in 10.0 ..= 500.0;
        /// Shell-impact flash (impact.fs): a one-sided punch plus a warm
        /// spark at the hit point - a sharp "thwack".
        impact_flash_duration: f32 = 0.14 in 0.01 ..= 2.0;
        /// Pulse growth, in standard field heights a second
        /// (`shockwave::RIPPLE_FRAME`).
        impact_flash_speed: f32 = 1.1 in 0.0 ..= 5.0;
        /// Thickness of the distorted band, in standard field heights.
        impact_flash_width: f32 = 0.02 in 0.0 ..= 0.5;
        /// How hard the pulse bends the image, in standard field heights.
        impact_flash_strength: f32 = 0.018 in 0.0 ..= 0.5;
        /// Half-extent (world px) of the impact flash's quad - it must hold
        /// the punch's whole reach, speed x duration plus the band in
        /// standard field heights of 544 px (about 95 px at the defaults),
        /// or the punch visibly clips.
        impact_flash_quad_radius: f32 = 130.0 in 10.0 ..= 500.0;
    }

    group shot_fx {
        /// One multiplier on the light every shot throws (`render/shot_fx.rs`,
        /// all additive and built from 2 px blocks): the halos and tracers
        /// in flight, the laser's bloom and flares, the missile's exhaust.
        /// 0 draws the plain sprites alone, with no muzzle flash.
        shot_glow_strength: f32 = 1.0 in 0.0 ..= 2.0;
        /// How many flat steps every glow is drawn in - shots, blasts,
        /// fires, fuses, towers, the pools of light on the ground - each
        /// step's edge dithered on the 2 px grid, as the weather's light
        /// pass steps the night (`pyro::glow`, docs/effects.md). 0 draws
        /// the smooth gradients instead.
        glow_bands: i32 = 4 in 0 ..= 12;
        /// Length (px) of the hot tracer streak a flying shell draws behind it.
        shell_tracer_length: f32 = 40.0 in 0.0 ..= 160.0;
        /// Length (px) of a minigun bullet's tracer streak.
        bullet_tracer_length: f32 = 26.0 in 0.0 ..= 160.0;
        /// Length (px) of the fading afterimage chain a plasma bolt leaves.
        plasma_trail_length: f32 = 52.0 in 0.0 ..= 160.0;
        /// Radius (px) of the plasma orb drawn in flight
        /// (`render/shot_shaders.rs`); its glow reaches about 2.3 times as far.
        plasma_orb_radius: f32 = 11.0 in 4.0 ..= 32.0;
        /// Turns per second of the plasma orb's surface, before each bolt's
        /// own speed factor (0.7 to 1.5, either way round).
        plasma_orb_spin_hz: f32 = 1.6 in 0.0 ..= 10.0;
        /// Size (px) of the light a muzzle flash throws at its first
        /// frame (`burst::muzzle`), which also sizes its tongue of fire;
        /// it steps down over `muzzle_flash_duration`.
        muzzle_glow_radius: f32 = 14.0 in 0.0 ..= 80.0;
        /// How fast (Hz) a laser beam's bloom and end flares flicker.
        laser_flicker_hz: f32 = 28.0 in 0.0 ..= 120.0;
        /// Sparks thrown off a hull, frog or border wall a shot hits
        /// (`fx.rs`, scaled by `fx_density`); tiles keep their own
        /// material bursts.
        shot_hit_sparks: i32 = 9 in 0 ..= 60;
        /// Sparks spat from the barrel with every shot (`fx.rs`, scaled
        /// by `fx_density`), plus a wisp of gun smoke.
        muzzle_sparks: i32 = 4 in 0 ..= 40;
        /// How long each hit plays (seconds; `burst.rs`): a shell's flash,
        /// fireball and smoke; a bullet's spark star; a plasma bolt's
        /// energy ring; a laser's molten splash; a tesla strike's crackling
        /// violet ring; a bio slush glob's splash of ooze.
        shell_hit_seconds: f32 = 0.6 in 0.05 ..= 3.0;
        bullet_hit_seconds: f32 = 0.22 in 0.05 ..= 2.0;
        plasma_hit_seconds: f32 = 0.5 in 0.05 ..= 3.0;
        laser_hit_seconds: f32 = 0.3 in 0.05 ..= 2.0;
        tesla_hit_seconds: f32 = 0.4 in 0.05 ..= 2.0;
        ooze_hit_seconds: f32 = 0.6 in 0.05 ..= 3.0;
        /// Size of every hit's burst, as a multiple of its designed size.
        hit_fx_scale: f32 = 1.0 in 0.2 ..= 3.0;
        /// Seconds the puff of dust a shot knocks off a tile it did not
        /// break takes to settle - stone, sand, sawdust or leaves in the
        /// tile's own colours (`burst.rs`).
        tile_dust_seconds: f32 = 0.7 in 0.05 ..= 3.0;
        /// Seconds the cloud a tile comes down in takes to roll out and
        /// settle.
        tile_collapse_seconds: f32 = 1.1 in 0.05 ..= 5.0;
        /// Seconds a hull or tile flashes in light when a shot lands on it,
        /// stepping down in three. 0 turns the flash off.
        hit_flash_seconds: f32 = 0.12 in 0.0 ..= 1.0;
        /// Glints per second a flying plasma bolt sheds in its own colour,
        /// and embers per second a flying shell sheds (`fx.rs`, scaled by
        /// `fx_density`).
        shot_trail_glint_rate: f32 = 26.0 in 0.0 ..= 200.0;
    }

    group fish {
        /// Fish in the lakes (`fish.rs`): cosmetic, read by nothing in the
        /// simulation. The most fish a school holds; each school takes a
        /// hashed count from half of this up. 0 leaves the water empty.
        fish_per_school: i32 = 5 in 0 ..= 16 @ Restart;
        /// Deep lake cells per school: a lake of this many open cells holds
        /// one, twice as many two. A lake under `fish_min_lake_cells` holds
        /// none.
        fish_school_cells: f32 = 40.0 in 4.0 ..= 1000.0 @ Restart;
        fish_min_lake_cells: i32 = 4 in 1 ..= 100 @ Restart;
        /// A fish's cruising pace and the pace it darts away at, px/s.
        fish_speed: f32 = 12.0 in 0.0 ..= 100.0;
        fish_dart_speed: f32 = 72.0 in 0.0 ..= 300.0;
        /// How near a hull has to come before the fish under the bank
        /// scatter, px from its centre.
        fish_scatter_px: f32 = 72.0 in 0.0 ..= 400.0;
        /// The same for a shot flying over the water or landing beside it
        /// (a hit, a ricochet, a laser's end).
        fish_shot_scatter_px: f32 = 40.0 in 0.0 ..= 400.0;
        /// The same for a blast, a missile's burst or a wreck going up.
        fish_blast_scatter_px: f32 = 168.0 in 0.0 ..= 800.0;
        /// How long a scared fish keeps darting before it settles back to
        /// its school.
        fish_calm_seconds: f32 = 2.5 in 0.1 ..= 20.0;
        /// How long a school keeps making for one spot of its lake before
        /// it picks another.
        fish_school_seconds: f32 = 7.0 in 0.5 ..= 60.0;
        /// Tail beats a second at cruising pace; a darting fish beats three
        /// times as fast.
        fish_tail_hz: f32 = 2.5 in 0.0 ..= 20.0;
        /// How strongly a fish shows through the water, stepped to eighths
        /// (`pyro::alpha`).
        fish_opacity: f32 = 0.75 in 0.0 ..= 1.0;
        /// Seconds, on average, between two fish carried down one column
        /// of a north/south stream. 0 sends none.
        fish_stream_seconds: f32 = 24.0 in 0.0 ..= 600.0;
    }

    group training {
        /// A wrecked seat in a training round (docs/training-stage.md)
        /// comes back as a fresh tank inside the last door opened, this
        /// long after it went (seconds). Training is never lost.
        training_respawn_seconds: f32 = 2.0 in 0.0 ..= 10.0;
        /// A training round's frog, once it is down, is back on its feet
        /// this long after (seconds) - the death animation's length - and
        /// the beat it fell in starts again.
        training_frog_revive_seconds: f32 = 1.6 in 0.0 ..= 10.0;
        /// An enemy wreck in a training round fades and is taken off the
        /// field this long after it went (seconds), so the course stays
        /// clear and no wreck keeps a gate's lane from the next beat's
        /// tank.
        training_wreck_seconds: f32 = 4.0 in 0.5 ..= 60.0;
        /// A beat's tank that has waited this long (seconds) for a free
        /// gate lane - a seat parked in it - drops onto the field out of
        /// sight instead, as a band round places one, so a beat never
        /// waits for ever.
        training_lane_wait_seconds: f32 = 5.0 in 0.5 ..= 60.0;
        /// A training beat's shot at the frog is fired again this long
        /// after the last (seconds) while the frog is still unhurt - the
        /// shell met a tank or a tile on the way - so a beat that waits on
        /// the frog's kit always gets a frog that needs it.
        training_frog_shot_retry_seconds: f32 = 3.0 in 0.5 ..= 30.0;
        /// How far past a flag's cell a hull still takes it, px: the
        /// hull's box grown by this much touches the flag's square.
        training_flag_reach_px: f32 = 6.0 in 0.0 ..= 32.0;
        /// How far each of the frog's hops takes it while it walks to its
        /// next beat's cell, as a share of its usual hop
        /// (`frog_hop_distance_factor`); it hops again as soon as it
        /// lands.
        training_frog_stride: f32 = 1.0 in 0.2 ..= 3.0;
        /// How long the frog's line stays up at the least (seconds), on
        /// top of `training_line_seconds_per_char` of its length
        /// (`bubble::FrogVoice`).
        training_line_seconds: f32 = 1.4 in 0.2 ..= 10.0;
        /// How much longer a line stays up for each of its letters
        /// (seconds), so a long line is up as long as it takes to read.
        training_line_seconds_per_char: f32 = 0.055 in 0.0 ..= 0.3;
        /// The pace the words of a line come into the bubble (words a
        /// second; the first is there at once).
        training_line_words_per_second: f32 = 9.0 in 1.0 ..= 60.0;
        /// Seconds with nothing said before the frog says the running
        /// beat's nudge again.
        training_nudge_seconds: f32 = 14.0 in 2.0 ..= 120.0;
    }

    group weather {
        /// Put one sky over every map, by its place in `map::Weather::ALL`:
        /// 0 clear, 1 night, 2 dusk, 3 rain, 4 storm, 5 fog, 6 sandstorm,
        /// 7 snow, 8 heat haze, 9 random (a sky picked by each round's
        /// seed). -1 plays each map's own `weather` key. `--weather` and
        /// the web page's `?weather=` set it at startup. A sky is settled
        /// when a round starts - the rules read it - so a change shows on
        /// the next one; a room's round is its map's, whatever this says.
        weather_override: i32 = (-1) in -1 ..= 9 @ Restart;
        /// One multiplier on every weather (docs/weather.md): the light
        /// eases toward daylight and every layer thins with it. 0 draws
        /// every sky clear, 1 as designed.
        weather_strength: f32 = 1.0 in 0.0 ..= 1.0;
        /// Let the sky change the rules (docs/weather.md "The rules"):
        /// shorter enemy sight at night, in a storm and in fog, less grip
        /// in the rain, the water frozen over in the snow, gusts in a
        /// sandstorm. Off, every sky is only drawn. The ice is laid when a
        /// round starts, so it follows this on the next one.
        weather_rules: bool = true in 0 ..= 1;
        /// How far an enemy sees at night and in a storm, as a fraction of
        /// `enemy_view_range`: the range it notices a player at, chases
        /// from and calls the others in from. An enemy never attacks past
        /// what it sees, so under `enemy_attack_range / enemy_view_range`
        /// this shortens its attack too. A hit still alerts it from
        /// anywhere.
        night_sight_factor: f32 = 0.6 in 0.1 ..= 1.0;
        /// The same in fog.
        fog_sight_factor: f32 = 0.45 in 0.1 ..= 1.0;
        /// The fraction of `tank_turn_grip_force` a hull keeps on wet
        /// ground in the rain and in a storm: it drifts further through a
        /// turn and a shove carries it further sideways. In a ford it
        /// multiplies `water_grip_factor`.
        rain_grip_factor: f32 = 0.5 in 0.05 ..= 1.0;
        /// On the ice a snowy sky freezes every lake and ford into: the
        /// fraction of `tank_turn_grip_force` a hull keeps (it slides
        /// through a turn), of `tank_accel_force` it gets (its tracks spin
        /// before it goes) and of `tank_decel_curve_rate` it brakes with
        /// (it coasts a long way). Top speed is kept.
        ice_grip_factor: f32 = 0.15 in 0.0 ..= 1.0;
        ice_traction_factor: f32 = 0.4 in 0.05 ..= 1.0;
        ice_brake_factor: f32 = 0.1 in 0.01 ..= 1.0;
        /// A sandstorm's gusts: now and then a wall of sand sweeps the
        /// field from the west and carries every hull it passes downwind -
        /// the water current's rule, the hull driving relative to the
        /// wind, so a stopped tank drifts and one driving upwind is held
        /// back. The wind's peak speed (px/s).
        sand_gust_speed: f32 = 48.0 in 0.0 ..= 300.0;
        /// The windows gusts come in (seconds): most windows have one,
        /// somewhere in their first half, and the round's first has none.
        sand_gust_gap_seconds: f32 = 9.0 in 2.0 ..= 60.0;
        /// How long a gust blows at any one point (seconds): it rises fast
        /// and dies away slowly.
        sand_gust_seconds: f32 = 1.5 in 0.2 ..= 10.0;
        /// How fast a gust's front crosses the field (px/s), and how far
        /// its heading swings off due east, either way (degrees).
        sand_gust_front_speed: f32 = 520.0 in 50.0 ..= 3000.0;
        sand_gust_spread_deg: f32 = 25.0 in 0.0 ..= 80.0;
        /// How bright full night is: the moonlight the whole field is lit
        /// by before any lamp, fire or shot adds to it (the blue tint is
        /// the look's own). A storm's gloom is half as bright again.
        night_ambient: f32 = 0.29 in 0.0 ..= 1.0;
        /// Steps per unit of light the light map is drawn in, on the 2 px
        /// block grid like every other glow; 0 draws smooth light.
        light_bands: i32 = 5 in 0 ..= 16;
        /// Dither between the light's steps with a 2x2 pattern, so a band
        /// edge reads as drawn rather than as a contour line.
        light_dither: bool = true in 0 ..= 1;
        /// Whether walls stop light: headlights, fires, portals and blasts
        /// cast shadows behind brick, iron, wood and the towers (glass
        /// lets it through; props and trees are too low or too open to).
        light_shadows: bool = true in 0 ..= 1;
        /// How far (px) a light carries into the wall that stops it, so the
        /// wall's near face is lit rather than a black edge.
        light_wall_bleed_px: f32 = 10.0 in 0.0 ..= 32.0;
        /// Headlights: how far ahead a hull's beam reaches (px; an enemy's
        /// reaches four fifths as far), half the cone's angle (degrees) and
        /// how bright it is.
        headlight_length_px: f32 = 190.0 in 0.0 ..= 600.0;
        headlight_half_angle_deg: f32 = 24.0 in 4.0 ..= 80.0;
        headlight_strength: f32 = 1.0 in 0.0 ..= 3.0;
        /// The glow every hull carries in its seat's colour (enemies a dim
        /// amber), so no tank is ever lost in the dark: radius (px) and
        /// strength.
        hull_glow_radius_px: f32 = 46.0 in 0.0 ..= 160.0;
        hull_glow_strength: f32 = 0.5 in 0.0 ..= 2.0;
        /// How much light shots, muzzle and impact flashes and hits throw
        /// into the dark.
        shot_light_strength: f32 = 1.0 in 0.0 ..= 3.0;
        /// Fire light - burning ground, burning tiles and wrecks, lit fuses:
        /// radius (px) and strength.
        fire_light_radius_px: f32 = 120.0 in 0.0 ..= 400.0;
        fire_light_strength: f32 = 1.0 in 0.0 ..= 3.0;
        /// The light a blast throws at its first frame (px, at a blast's
        /// scale 1); a mushroom cloud's reaches half as far again.
        blast_light_radius_px: f32 = 210.0 in 0.0 ..= 600.0;
        /// Pickups and frogs glow faintly, so a lamp can find them.
        pickup_glow_strength: f32 = 0.4 in 0.0 ..= 2.0;
        /// Rain: amount (a multiplier on the look's), fall speed (px/s),
        /// slant (x px per px fallen; negative leans the other way) and
        /// how often drops splash on the ground and ring the water.
        rain_density: f32 = 1.0 in 0.0 ..= 2.0;
        rain_speed_px: f32 = 640.0 in 50.0 ..= 2000.0;
        rain_slant: f32 = 0.24 in -1.0 ..= 1.0;
        rain_splash_rate: f32 = 1.0 in 0.0 ..= 3.0;
        /// Average seconds between lightning strikes in a storm, and how
        /// bright a strike lights the field.
        lightning_gap_seconds: f32 = 7.0 in 1.0 ..= 60.0;
        lightning_strength: f32 = 1.0 in 0.0 ..= 2.0;
        /// Fog: amount (a multiplier on the look's) and how fast its banks
        /// drift.
        fog_density: f32 = 1.0 in 0.0 ..= 2.0;
        fog_drift_speed: f32 = 1.0 in 0.0 ..= 5.0;
        /// Fog and blowing sand thin out within this radius (px) of every
        /// seat's tank, so a player always sees their own ground. 0 draws
        /// the air the same everywhere.
        weather_clear_radius_px: f32 = 120.0 in 0.0 ..= 400.0;
        /// Sandstorm: amount (a multiplier on the look's) and wind speed.
        sand_density: f32 = 1.0 in 0.0 ..= 2.0;
        sand_wind_speed: f32 = 1.0 in 0.0 ..= 4.0;
        /// Snow: how much falls (a multiplier on the look's) and how much
        /// of the ground it covers, 0 to 1.
        snow_density: f32 = 1.0 in 0.0 ..= 2.0;
        snow_cover: f32 = 1.0 in 0.0 ..= 1.0;
        /// Heat haze: how far (px) rows of the field shimmer - in whole
        /// 2 px steps, so the art never smears - and how fast.
        haze_amplitude_px: f32 = 2.0 in 0.0 ..= 8.0;
        haze_speed: f32 = 1.0 in 0.0 ..= 4.0;
        /// How much a heavy sky darkens the field toward its edges (a
        /// multiplier on the look's own vignette).
        weather_vignette: f32 = 1.0 in 0.0 ..= 2.0;
        /// Draw every sky the way a device whose GPU would not compile the
        /// weather's shaders draws it (docs/weather.md "Without shaders"):
        /// the light map multiplied in by a blend mode, the snow on the
        /// ground and the fog, sand, rain and snow in the air as plain
        /// blocks. For looking at that picture where the shaders work.
        weather_without_shaders: bool = false in 0 ..= 1;
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

    /// The sight box's half extents in world px, (sideways, up and down):
    /// `sight_box_half_cols` and `sight_box_half_rows` cells of
    /// `OBSTACLE_GRID_SIZE` - +-368 x +-240 at the defaults.
    pub fn sight_box_half_px(&self) -> (f32, f32) {
        (self.sight_box_half_cols * crate::OBSTACLE_GRID_SIZE, self.sight_box_half_rows * crate::OBSTACLE_GRID_SIZE)
    }

    /// Fire cooldown held for a whole minigun burst: every queued bullet's
    /// delay plus the trailing gap.
    pub fn minigun_burst_cooldown_seconds(&self) -> f32 {
        (self.minigun_burst_size.saturating_sub(1)) as f32 * self.minigun_bullet_delay_seconds
            + self.minigun_burst_trailing_gap_seconds
    }

    /// Missiles one trigger pull fires: every salvo's tubes.
    pub fn missile_volley_count(&self) -> u32 {
        self.missile_volley_size.clamp(1, 4) * self.missile_salvos.max(1)
    }

    /// Fire cooldown held for a whole missile volley: every queued
    /// launch's delay, the gaps between salvos, then the pod's reload.
    pub fn missile_volley_cooldown_seconds(&self) -> f32 {
        let salvos = self.missile_salvos.max(1);
        let launches = self.missile_volley_count().saturating_sub(salvos);
        launches as f32 * self.missile_launch_delay_seconds
            + (salvos - 1) as f32 * self.missile_salvo_gap_seconds
            + self.missile_reload_seconds
    }

    /// Schema row for `name`, if it's a table row.
    pub fn meta(name: &str) -> Option<&'static ParamMeta> {
        Self::SCHEMA.iter().find(|m| m.name == name)
    }

    /// This table with a JSON patch object applied to a copy, or the
    /// error that rejected it whole - the one place a patch string
    /// becomes a table, shared by [`submit_json`] (the window's side of a
    /// room's `Welcome`, the dev panel, `--tuning`) and by anyone
    /// checking what a patch would do without touching the global store.
    pub fn with_json_patch(&self, json: &str) -> Result<Tuning, String> {
        let patch: Value = serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))?;
        let Value::Object(patch) = patch else {
            return Err("expected a JSON object of {\"knob\": value} pairs".to_string());
        };
        let mut next = *self;
        next.apply_patch(&patch)?;
        Ok(next)
    }

    /// Apply a JSON patch object: any subset of keys, each a number (scalar
    /// row or `name.elem` element), a bool, or - for an array row - a
    /// full-length array. Validated key by key; the caller is expected to
    /// apply this to a *copy* and only commit on `Ok`, which is what
    /// [`with_json_patch`](Tuning::with_json_patch) does. Returns how many
    /// keys were applied.
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

/// The live table, as a shared snapshot: a reader clones the `Arc` under
/// the lock and lets go at once, and a writer swaps a new `Arc` in.
///
/// **No code ever runs while the lock is held**, and that is the point.
/// The table used to hand out the read guard itself, and the simulation
/// binds `let t = tuning()` at the top of a phase and then calls helpers
/// that call `tuning()` again - a second read while holding the first.
/// `std`'s `RwLock` lets a waiting writer block new readers, so a write
/// arriving between the two (a client applying its room's tuning patch,
/// the room server's `init_under` starting a round in another room)
/// waited for the first guard while the second waited for the writer:
/// a deadlock that froze every thread that read tuning - in the room
/// server, every room. A snapshot has nothing to wait for.
static TUNING: LazyLock<RwLock<Arc<Tuning>>> = LazyLock::new(|| RwLock::new(Arc::new(Tuning::DEFAULT)));
/// The next table, built up by `submit_*` calls since the last frame
/// boundary; `apply_pending` swaps it in. Staging on a copy means a batch of
/// submits between two frames all land together, and a rejected patch never
/// half-applies.
static STAGED: Mutex<Option<Tuning>> = Mutex::new(None);
static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The live table, as a snapshot: cheap (a read lock held for one `Arc`
/// clone), and safe to hold for as long as the caller likes - across other
/// `tuning()` calls and across a write on another thread, which the
/// snapshot simply does not see. Bind once per function in hot code.
#[inline]
pub fn tuning() -> Arc<Tuning> {
    Arc::clone(&TUNING.read().unwrap_or_else(PoisonError::into_inner))
}

/// Swap `next` in as the live table.
fn set_live(next: Tuning) {
    *TUNING.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(next);
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
            set_live(next);
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
    set_live(t);
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
        assert_eq!(t.sight_box_half_px(), (368.0, 240.0));
        t.enemy_attack_range = 500.0;
        assert_eq!(t.engage_ring_radius(), 400.0);
        t.sight_box_half_rows = 8.0;
        assert_eq!(t.sight_box_half_px(), (368.0, 256.0));
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
