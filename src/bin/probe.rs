//! Headless gameplay-mechanics probe: drives `simulation::Game` frame by
//! frame with a scripted `Input` sequence and no window/renderer, then dumps
//! tank state as text - the "feedback loop" for verifying/exploring
//! mechanics (movement, damage, AI behavior, round outcome) without
//! eyeballing the actual game. See CLAUDE.md's "Testing & tooling" section.
//!
//! Deterministic given a seed: `Game::init` seeds the whole round's one
//! RNG stream (see `Game::rng` and docs/gameplay-verification-design.md),
//! so the same `--seed` replays a round bit-for-bit. Every `ANOMALY` line
//! carries its round's own `seed=0x...`, ready to paste straight back into
//! `--seed` - here for a frame-level trace (`--log-every 1`), or into the
//! game binary's own `--seed` to watch that exact layout play out (same
//! setup, though windowed evolution diverges over time under variable
//! frame dt). Unseeded runs draw and print a random base seed, so they're
//! just as replayable after the fact. The guard keeping all of this true
//! is `simulation::determinism_tests`; promote a nailed-down scenario to a
//! `#[cfg(test)]` there once it's worth locking in as a regression check.
//!
//! Besides the scripted per-frame trace (`log_frame`/`--log-every`), this
//! also runs a set of standing anomaly checks (`AnomalyTracker`) against
//! every enemy tank every frame, printing a single greppable `ANOMALY` line
//! the moment one trips instead of requiring a human (or an LLM) to read a
//! full frame-by-frame trace looking for it. `--rounds N` re-`init`s the
//! game N times (fresh spawn layout/AI RNG each time, same as the game's own
//! restart) to get statistical coverage of rare, spawn-dependent bugs in one
//! command - see the "Capturing gameplay anomalies" section of CLAUDE.md.

use bongbong::tuning::tuning;
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bongbong::Position;
use bongbong::ai::Intent;
use bongbong::map::MapFile;
use bongbong::simulation::{Game, Input, Outcome, TankSnapshot};
use bongbong::tank::Dir;
use bongbong::{
    DEFAULT_SCREEN_HEIGHT,
    DEFAULT_SCREEN_WIDTH,
    PATHFIND_CELL_SIZE,
};
use clap::{Parser, ValueEnum};
use rand::RngExt;

/// Same `-m`/`--map` fallback the real game applies (main.rs) - the
/// embedded default map, so a probe run exercises the same battlefield the
/// game actually ships with, rather than the empty `MapFile::default()`
/// `Game::default()` would otherwise leave in place. Embedded via
/// `include_str!` rather than read from disk for the same reason main.rs's
/// own `default_map()` is - see `MapFile::from_toml_str`'s doc comment.
fn default_map() -> MapFile {
    MapFile::from_toml_str(include_str!("../../maps/default.toml"))
        .expect("failed parsing the embedded default map")
}

/// A `--map` value: the parsed map plus the path it came from, kept for
/// the run header (main.rs's own `parse_map` drops the path because the
/// game never displays it; sweep output should say which battlefield the
/// numbers belong to). Parsed eagerly at CLI-parse time like main.rs's -
/// a missing/malformed file fails fast with a clear error instead of
/// after a sweep has already started.
#[derive(Clone)]
struct NamedMap {
    name: String,
    map: MapFile,
}

fn parse_map(s: &str) -> Result<NamedMap, String> {
    MapFile::load(Path::new(s)).map(|map| NamedMap { name: s.to_string(), map })
}

/// One `--budget` cap: an anomaly kind (an ANOMALY-line tag, or `total`)
/// and the maximum count a run may produce before exiting 1 - see
/// `check_budgets` and docs/gameplay-verification-design.md §6.2.
#[derive(Clone)]
struct Budget {
    kind: String,
    max: u32,
}

fn parse_budget(s: &str) -> Result<Budget, String> {
    let valid = || format!("{}, total", ANOMALY_KINDS.join(", "));
    let (kind, max) = s
        .split_once('=')
        .ok_or_else(|| format!("expected <kind>=<max> (e.g. stall=0); kinds: {}", valid()))?;
    let kind = kind.trim();
    if kind != "total" && !ANOMALY_KINDS.contains(&kind) {
        return Err(format!("unknown anomaly kind '{kind}'; kinds: {}", valid()));
    }
    let max = max
        .trim()
        .parse()
        .map_err(|e| format!("invalid max '{}': {e}", max.trim()))?;
    Ok(Budget { kind: kind.to_string(), max })
}

// The battlefield dimensions every probe round runs at - derived from the
// game's own default window size (lib.rs) so the two can't drift; the
// probe deliberately has no `--resolution` counterpart (one canonical size
// keeps sweep numbers comparable across runs).
const WIDTH: f32 = DEFAULT_SCREEN_WIDTH as f32;
const HEIGHT: f32 = DEFAULT_SCREEN_HEIGHT as f32;
const DT: f32 = 1.0 / 60.0;
/// See `Scenario::Brake`'s doc comment for why this is short.
const BRAKE_HOLD_FRAMES: u32 = 18;

// --- Anomaly-detection tuning ---
// A tank must get at least this far from its spawn point at some point
// within STALE_START_FRAMES or it's flagged as never having left spawn.
// Judged on the *farthest* it got, not where it happens to be when the
// window closes: a tank that drove off and wandered back is not stale.
const STALE_START_FRAMES: u32 = 120; // 2s
const STALE_START_EPS: f32 = 5.0; // px
// After the stale-start window, a tank sitting near-zero speed for this many
// consecutive frames (while the round is still Playing and it isn't a
// wreck) is flagged as stalled out mid-round.
const STALL_SPEED_EPS: f32 = 5.0; // px/s - top speeds are 150-220 (tank_speed/enemy_speed in tuning.rs)
const STALL_FRAMES_THRESHOLD: u32 = 180; // 3s
// Standing still is only an anomaly when the tank has no fighting reason to
// hold position - `act_attack` deliberately stops to aim/settle/fire, and
// `act_retreat` deliberately parks outside ENEMY_RETREAT_RANGE to wait out
// the shell recharge. `TankTrack::deliberate_hold` (see its comment for the
// three conditions, each tied to the ai.rs behavior it mirrors) resets the
// stall counter and vetoes the stale-start flag, so those two kinds only
// catch pathological stillness, not the AI correctly executing a firing
// solution from where it already stands. Added after the pathfind-grid fix:
// once enemies could actually reach firing positions, an AFK player started
// dying in ~6s to tanks that (correctly) never needed to move, and every
// one of them lit up stale-start/stall as a false positive.
// A shot is detected as any ammo pool decreasing between frames (shell/
// minigun/plasma/laser - recharge only ever moves shells *up*, so a
// decrease is always a trigger pull); "fired recently" means within this
// trailing window:
const FIRED_RECENTLY_FRAMES: u32 = STALL_FRAMES_THRESHOLD;
// The battlefield walls' inner faces sit exactly at 0/width/0/height
// (spawn_walls, simulation.rs) - a tank whose center stays within this
// margin of one of those four lines for this many consecutive frames is
// flagged as stuck against the border.
const BORDER_MARGIN: f32 = 40.0; // px
const BORDER_FRAMES_THRESHOLD: u32 = 90; // 1.5s
// Facing jitter: an A -> B -> A heading flip-flop ("committed_dir"/
// "dir_hold" in ai.rs exist specifically to prevent this near 45-degree
// diagonals) counted within a trailing window; JITTER_THRESHOLD flips
// inside JITTER_WINDOW_FRAMES flags it.
const JITTER_WINDOW_FRAMES: u32 = 120; // 2s
const JITTER_THRESHOLD: u32 = 4;
// Spin: a full circle of *same-direction* quarter-turns (U->R->D->L->U or
// the mirror) completed inside the window while going nowhere - the "tank
// spins in place" failure the jitter check is structurally blind to (a
// rotational cycle contains no A,B,A triple). The visible symptom in the
// windowed game is the hull/turret perpetually turning: both visual eases
// (TANK_VISUAL_TURN_SPEED_DEG / TANK_TURRET_VISUAL_TURN_SPEED_DEG) only
// ever chase `Tank::rotation`, so a spinning sprite always means the
// sim-side heading itself is cycling - which is what's watched here.
// Legit navigation can't trip it: rounding even the smallest obstacle
// block takes a ~430px loop (>2s at ENEMY_SPEED), so completing 360 inside
// the window with under SPIN_NET_MAX of net drift means the turns came
// from re-decisions, not a path. A 180 reversal breaks the chain (that's
// jitter/churn territory, not rotation).
const SPIN_WINDOW_FRAMES: u32 = 120; // 2s
const SPIN_FULL_CIRCLE_DEG: f32 = 360.0;
const SPIN_NET_MAX: f32 = 60.0; // px of net drift allowed over the circle
// Churn: lots of driving, no progress - net displacement over a trailing
// window stays under CHURN_NET_MAX despite CHURN_MIN_PATH of actual path
// traveled ("dancing"/back-and-forth). The complement of `stall` (which
// needs near-zero speed) and of `spin` (which needs rotation): a tank
// oscillating between two headings at full speed trips neither. Thresholds:
// 300px over 4s is an average of 75px/s - half of ENEMY_SPEED, so the tank
// really was being driven most of the window, not just nudged - while
// ending up less than ~1.5 tank-lengths from where it started.
const CHURN_WINDOW_FRAMES: u32 = 240; // 4s
const CHURN_MIN_PATH: f32 = 300.0; // px of path traveled within the window
const CHURN_NET_MAX: f32 = 60.0; // px of net displacement
// --- Contact-metric kinds (docs/gameplay-verification-design.md §4.3) ---
// Built on TankSnapshot's narrow-phase read-backs (touching_static /
// contact_impulse / commanded_velocity / top_speed - see
// Physics::contact_stats): these observe real solver contacts, where every
// kind above infers trouble from kinematics alone.
// Wall-grind: driving *into* terrain - commanded to move while the solid
// hull holds an active contact with static geometry (wall, obstacle tile,
// or the frog), sustained. The failure `stall` is structurally blind to:
// sliding along the wall being ground against can keep speed well above
// STALL_SPEED_EPS the whole time. No deliberate_hold interplay needed - a
// holding tank has no move command, and GRIND_CMD_EPS requires one.
const GRIND_FRAMES: u32 = 120; // 2s, matching STALE_START_FRAMES's scale
const GRIND_CMD_EPS: f32 = 1.0; // px/s of commanded speed = "being driven"
// Bump-rate: rising edges of touching_static inside a trailing minute -
// "hitting obstacles too much" made literal, catching repeated brief
// collisions that never individually last GRIND_FRAMES. Threshold set from
// measured per-tank window maxima (30s afk/advance/circle rounds, 4
// enemies, --seed 1000, 10 rounds each, 2026-08-27): default map 7/13/12
// (afk/advance/circle - the busy shipped map is the contact-heaviest),
// every maps/test fixture 0-5. Cap = 30: ~2.3x the worst legitimate peak
// even allowing a full-minute window to roughly double a 30s round's
// sustained rate, while classic collide-back-off thrash (a bump every
// 1-2s) runs 30-60/min and trips it inside the window.
const BUMP_WINDOW_FRAMES: u32 = 3600; // 60s
const BUMP_RATE_MAX: u32 = 30;
// Low-progress: intent vs outcome - commanded at least half of top speed
// while the body achieves under a third of what was commanded, sustained.
// The principled "stuck": catches half-speed grinding that clears
// STALL_SPEED_EPS. The window is ~4x ai.rs's own STUCK_ESCAPE_SECONDS
// (0.75s at <STUCK_SPEED_EPS px/s), so this fires only when that escape
// hatch keeps triggering and keeps failing, never on one normal trigger.
const PROGRESS_FRAMES: u32 = 180; // 3s, matching STALL_FRAMES_THRESHOLD
const PROGRESS_CMD_FRACTION: f32 = 0.5; // commanded >= this * top_speed
const PROGRESS_ACHIEVED_FRACTION: f32 = 0.3; // real < this * commanded
// Clustering: enemies pathing to the same point (or funneling through the
// same chokepoint) piling up on top of each other instead of spreading out -
// see the claim-based engagement-slot system in simulation.rs::Game::update
// (ENGAGE_RING_RADIUS/ENGAGE_LATERAL_OFFSET/ENGAGE_RESERVE_RADIUS in lib.rs),
// added specifically to fix this. Not a mutual clique - it's "this enemy has
// at least CLUSTER_MIN_GROUP - 1 other live enemies within CLUSTER_RADIUS of
// itself", sustained for CLUSTER_FRAMES_THRESHOLD, not just a momentary
// crossing. The current geometry's tightest steady-state grouping is a
// same-axis firing pair, 2 * ENGAGE_LATERAL_OFFSET (36px) apart - only 2
// tanks, one short of CLUSTER_MIN_GROUP - with the nearest third point
// either the reserve rank (ENGAGE_RESERVE_RADIUS - ENGAGE_RING_RADIUS =
// 128px behind it) or an adjacent axis's pair (~385px away), both well
// outside CLUSTER_RADIUS. So a sustained 3-within-90px reading is still a
// genuine failure, not a side effect of the geometry itself.
const CLUSTER_RADIUS: f32 = bongbong::simulation::debug::CLUSTER_RADIUS_PX; // px between tank centers, shared with the live snapshot
const CLUSTER_MIN_GROUP: usize = 3; // this many mutually-close enemies counts as a cluster
const CLUSTER_FRAMES_THRESHOLD: u32 = 180; // 3s, matching STALL_FRAMES_THRESHOLD's window
// --- Navigation e2e: path-stretch (docs/gameplay-verification-design.md §5.2) ---
// never-arrived: a live enemy that had a route to the player at round
// start (Game::nav_path_cells - the same nav grid + A* the AI steers by)
// but still hasn't come within ENEMY_ATTACK_RANGE of the player by the
// time the round ends, despite far more time than that route needs.
// Engagement range - not contact - is the right goal: crossing it is the
// moment act_attack's stop-and-fight behavior takes over and approaching
// legitimately ends. The time budget is NAV_GRACE_SECONDS (patrol wander
// plus alert propagation before pursuit even starts) plus NAV_STRETCH_MAX
// x ideal_seconds, where ideal = path_cells * PATHFIND_CELL_SIZE /
// top_speed is a straight-route-driving floor: dodging, heading
// commitment, and engagement-slot detours legitimately cost 2-3x, so 4x
// flags only genuine routing failure. Checked once, when the round ends
// (or hits the frame cap) - and the flat grace term means a round that
// ends quickly (post-grid-fix an AFK player can be dead inside ~6s) can
// never false-flag: elapsed stays under NAV_GRACE_SECONDS alone.
const NAV_GRACE_SECONDS: f32 = 10.0;
const NAV_STRETCH_MAX: f32 = 4.0;
// --- Frame-invariant sanity bounds (kind=invariant) ---
// Physics sanity rather than behavior: any violation is a hard bug (solver
// explosion, tunneling through the boundary walls, NaN poisoning) with the
// printed seed as a perfect repro. Checked for *every* tank, player
// included - unlike the behavior checks, a physics blow-up on the player
// isn't explained by the scripted Scenario. Bounds are generous on purpose
// so legitimate gameplay can never trip them: the position margin sits
// outside the walls' inner faces (0/WIDTH/0/HEIGHT, see spawn_walls) but
// inside their WALL_THICKNESS-padded outer extent, and the speed cap is
// well above TANK_SPEED (220px/s) times SPEED_BOOST_MULTIPLIER plus any
// legitimate ram/explosion knockback spike.
const INVARIANT_POS_MARGIN: f32 = 50.0; // px outside the walls' inner faces
const INVARIANT_SPEED_MAX: f32 = 800.0; // px/s

#[derive(Clone, Copy, ValueEnum)]
enum Scenario {
    /// Player never moves or fires - watch the AI find and attack a passive
    /// target from a standing start.
    Afk,
    /// Player advances straight up the battlefield, firing every half
    /// second - watch engagement/damage exchange play out.
    Advance,
    /// Player drives Up just long enough to reach top speed
    /// (`BRAKE_HOLD_FRAMES`), then releases for the rest of the run - watch
    /// on-axis velocity decay to verify/tune the TANK_DECEL_CURVE_RATE
    /// braking curve (see lib.rs). Deliberately short: total travel before
    /// and during the brake stays within OBSTACLE_CLEAR's guaranteed-empty
    /// 90px radius around the player's spawn point, so the random map
    /// layout can't put an obstacle in the way and contaminate the reading
    /// with a collision stop. Note: an all-enemies-wrecked check runs every
    /// frame and is vacuously true with 0 enemies, ending the round
    /// instantly - don't pass `--enemies 0` (an enemy that far away can't
    /// reach the player in this short a window anyway). Run with
    /// `--frames 60 --log-every 1` to see the curve frame-by-frame.
    Brake,
    /// Player drives a continuous square loop (1s per leg: Up, Right, Down,
    /// Left) without firing - a perpetually moving target whose bearing from
    /// every enemy keeps sweeping across the 45-degree diagonals and whose
    /// engagement-ring slots (simulation.rs) keep rotating with it. Built to
    /// provoke heading-churn failures - rapid hull re-commits that read in
    /// the windowed game as tanks spinning in place or dancing
    /// back-and-forth instead of cleanly repositioning (see the `spin` and
    /// `churn` anomaly kinds) - which a stationary (`afk`) or
    /// steadily-receding (`advance`) player mostly fails to trigger.
    Circle,
}

#[derive(Parser)]
#[command(name = "probe", about = "Headless bongbong gameplay-mechanics probe")]
struct Args {
    /// Which scripted scenario to run.
    #[arg(long, value_enum, default_value_t = Scenario::Afk)]
    scenario: Scenario,

    /// How many enemies to spawn (default: same random range as the real game).
    #[arg(short = 'e', long)]
    enemies: Option<usize>,

    /// Override the map's mission: protect, hunt or destroy (see
    /// docs/maps-to-levels.md). Note a hunt round against an AFK player is
    /// expected to be lost quickly.
    #[arg(long, value_enum)]
    mission: Option<bongbong::level::Mission>,

    /// Override the map's spawn plan: band or waves.
    #[arg(long, value_enum)]
    spawn: Option<bongbong::level::SpawnKind>,

    /// Waves plan: number of waves.
    #[arg(long)]
    waves: Option<u32>,

    /// Waves plan: tanks in the first wave.
    #[arg(long)]
    wave_size: Option<u32>,

    /// Waves plan: tanks added per wave.
    #[arg(long)]
    wave_growth: Option<u32>,

    /// Waves plan: chassis tier of the first wave.
    #[arg(long, value_enum)]
    tier_start: Option<bongbong::level::Tier>,

    /// Waves plan: chassis tier of the last wave.
    #[arg(long, value_enum)]
    tier_end: Option<bongbong::level::Tier>,

    /// Maximum frames to simulate per round before giving up (default: 3600 = 60s at 60fps).
    #[arg(long, default_value_t = 3600)]
    frames: u32,

    /// Print a state snapshot every N frames (default: 60 = once per simulated second).
    /// Ignored (no per-frame trace) when --rounds > 1 - see that flag.
    #[arg(long, default_value_t = 60)]
    log_every: u32,

    /// Re-run the scenario this many times, each with a fresh `init` (new
    /// spawn layout and AI RNG roll), to get statistical coverage of rare,
    /// spawn-dependent anomalies in one command. With more than one round,
    /// per-frame `log_frame` snapshots are suppressed - only ANOMALY lines
    /// and a one-line per-round result print, plus a final summary. Anomaly
    /// checks always run regardless of this flag; --rounds just controls
    /// how many independent layouts get checked.
    #[arg(long, default_value_t = 1)]
    rounds: u32,

    /// Pin the base RNG seed (decimal or 0x-hex) for exact replay. Round i
    /// of a run uses base+i, so a `--rounds` sweep stays one command while
    /// every round keeps its own printed `seed=0x...` - paste that value
    /// back here (with the default `--rounds 1`) to replay just that round,
    /// or into the game binary's `--seed` to watch it. Unseeded runs draw
    /// a random base and print it, so they're replayable after the fact
    /// too.
    #[arg(long, value_parser = bongbong::parse_seed)]
    seed: Option<u64>,

    /// Load a tuning patch (JSON object of `{"knob": value}` pairs - see
    /// docs/runtime-tuning-design.md) before the first round, so a sweep
    /// can measure a candidate tuning set headlessly. Applied once, up
    /// front; `(seed, tuning diff)` is the replay pair, and the diff is
    /// echoed in the run header and each `--json-out` record.
    #[arg(long = "tuning")]
    tuning: Option<std::path::PathBuf>,

    /// With `--tuning`: print the loaded patch as `tunables!` table rows
    /// (the same text as the web panel's "Copy as Rust") and exit, for
    /// pasting over the matching rows in src/tuning.rs to make a QA'd set
    /// the new default. Runs no rounds.
    #[arg(long = "print-rust", requires = "tuning")]
    print_rust: bool,

    /// Battlefield map to probe (same `-m`/`--map` semantics as the game
    /// binary, loaded and validated eagerly); defaults to the embedded
    /// `maps/default.toml` the game itself ships with. Point it at a
    /// `maps/test/` fixture to sweep one adversarial layout - see
    /// docs/gameplay-verification-design.md §2.2 for what each fixture is
    /// built to provoke.
    #[arg(short = 'm', long = "map", value_parser = parse_map)]
    map: Option<NamedMap>,

    /// Write one JSON object per round (JSON Lines) to this file,
    /// overwriting it - the machine-readable counterpart of the human
    /// stdout (which is unchanged), for scripts that rank worst seeds,
    /// plot stretch distributions, or diff two branches' sweeps. Schema is
    /// versioned ("v":1); every line carries its round's seed ready to
    /// paste back into --seed. See docs/gameplay-verification-design.md
    /// §6.1.
    #[arg(long = "json-out")]
    json_out: Option<PathBuf>,

    /// Fail the run (exit code 1, after a `BUDGET EXCEEDED` line per
    /// breach) if a kind's total exceeds its cap - repeatable, e.g.
    /// `--budget stall=0 --budget total=5`. Kinds are the ANOMALY-line
    /// tags plus `total`; with no budgets the exit code stays 0 always,
    /// today's behavior. This is the CI gate (§6.2): budgets encode a
    /// *measured* baseline being ratcheted down, never aspirations - set
    /// them from recorded sweep numbers, and never raise one just to get
    /// past a failure.
    #[arg(long = "budget", value_parser = parse_budget)]
    budgets: Vec<Budget>,

    /// After the run, print an ASCII battlefield heatmap per anomaly kind
    /// that fired (one character per PATHFIND_CELL_SIZE nav cell, same
    /// grid the AI routes on): turns "7 stalls somewhere" into "the stalls
    /// are all in the two cells beside the choke mouth" (§6.3).
    #[arg(long)]
    heatmap: bool,
}

fn input_for_frame(scenario: Scenario, frame: u32) -> Input {
    let mut player_intent = Intent::default();
    match scenario {
        Scenario::Afk => {}
        Scenario::Advance => {
            player_intent.move_dir = Some(Dir::Up);
            player_intent.fire = frame % 30 == 0;
        }
        Scenario::Brake => {
            if frame <= BRAKE_HOLD_FRAMES {
                player_intent.move_dir = Some(Dir::Up);
            }
        }
        Scenario::Circle => {
            player_intent.move_dir = Some(match (frame / 60) % 4 {
                0 => Dir::Up,
                1 => Dir::Right,
                2 => Dir::Down,
                _ => Dir::Left,
            });
        }
    }
    Input {
        player_intent,
        pause_pressed: false,
        restart_pressed: false,
        toggle_shadows_pressed: false,
        cycle_overlays_pressed: false,
    }
}

fn outcome_str(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Playing => "Playing",
        Outcome::Won => "Won",
        Outcome::Lost => "Lost",
    }
}

fn log_frame(game: &Game, frame: u32) {
    println!(
        "[t={:6.2}s frame={frame:5}] outcome={}",
        frame as f32 * DT,
        outcome_str(game.outcome())
    );
    let mut enemy_index = 0;
    for tank in game.tank_snapshots() {
        let label = if tank.is_player {
            "PLAYER ".to_string()
        } else {
            let label = format!("ENEMY#{enemy_index}");
            enemy_index += 1;
            label
        };
        let speed = (tank.velocity.x * tank.velocity.x + tank.velocity.y * tank.velocity.y).sqrt();
        println!(
            "  {label} pos=({:6.1},{:6.1}) vel=({:6.1},{:6.1}) speed={:6.1} rot={:5.0} dmg={:5.1}/100 ammo={:2} plasma={:2} minigun={:3} laser={:2} shield={:4.1} wreck={}",
            tank.position.x, tank.position.y, tank.velocity.x, tank.velocity.y, speed, tank.rotation, tank.damage, tank.shells_ammo, tank.plasma_ammo, tank.minigun_ammo, tank.laser_charges, tank.shield_timer, tank.is_wreck,
        );
    }
}

#[derive(Default, Clone, Copy)]
struct AnomalyTotals {
    stale_start: u32,
    stall: u32,
    border_stuck: u32,
    jitter: u32,
    spin: u32,
    churn: u32,
    clustering: u32,
    wall_grind: u32,
    bump_rate: u32,
    low_progress: u32,
    never_arrived: u32,
    invariant: u32,
}

/// Canonical anomaly-kind tags in reporting order - exactly the strings
/// `report` prints in ANOMALY lines. The single source of truth wiring a
/// kind into `--budget` validation, the `--json-out` `anomalies` object
/// (tags with `-` swapped for `_`), and the `--heatmap` per-kind ordering:
/// a new kind added here and in `AnomalyTotals::count` reaches all three
/// automatically.
const ANOMALY_KINDS: [&str; 12] = [
    "stale-start",
    "stall",
    "border-stuck",
    "jitter",
    "spin",
    "churn",
    "clustering",
    "wall-grind",
    "bump-rate",
    "low-progress",
    "never-arrived",
    "invariant",
];

impl AnomalyTotals {
    fn total(&self) -> u32 {
        ANOMALY_KINDS.iter().map(|kind| self.count(kind)).sum()
    }

    /// This run's count for one `ANOMALY_KINDS` tag - the string-keyed
    /// read `--budget`/`--json-out` need, kept next to the field list so
    /// the two can't drift.
    fn count(&self, kind: &str) -> u32 {
        match kind {
            "stale-start" => self.stale_start,
            "stall" => self.stall,
            "border-stuck" => self.border_stuck,
            "jitter" => self.jitter,
            "spin" => self.spin,
            "churn" => self.churn,
            "clustering" => self.clustering,
            "wall-grind" => self.wall_grind,
            "bump-rate" => self.bump_rate,
            "low-progress" => self.low_progress,
            "never-arrived" => self.never_arrived,
            "invariant" => self.invariant,
            _ => unreachable!("unknown anomaly kind '{kind}' - not in ANOMALY_KINDS"),
        }
    }
}

/// Per-enemy-tank state carried across frames within one round, for the
/// anomaly checks in `AnomalyTracker::check`. Indexed in lockstep with
/// `Game::tank_snapshots()`'s iteration order, which is stable for the
/// whole round: tanks are never despawned (a wreck stays in `world`, just
/// inert - see simulation.rs), only shells/obstacles are, so the same slot
/// is always the same tank.
struct TankTrack {
    label: String,
    spawn_pos: Position,
    // Farthest the tank has been from `spawn_pos` so far (stale-start).
    max_spawn_dist: f32,
    stall_frames: u32,
    border_frames: u32,
    stale_flagged: bool,
    stall_flagged: bool,
    border_flagged: bool,
    jitter_flagged: bool,
    cluster_frames: u32,
    cluster_flagged: bool,
    // Last up to 3 *distinct* headings seen, oldest first - an A,B,A pattern
    // here is a genuine flip-flop, not just "still facing the same way".
    heading_history: VecDeque<f32>,
    // Frame numbers of recent A,B,A flip-flop events, for the trailing
    // JITTER_WINDOW_FRAMES count.
    recent_flips: VecDeque<u32>,
    // --- spin: a chain of consecutive same-direction quarter-turns ---
    spin_flagged: bool,
    // The heading as of the previous frame, so a change (and its signed
    // 90-degree delta) is detected the frame it happens.
    last_heading: f32,
    // Signed degrees accumulated by the current same-direction turn chain
    // (positive = clockwise); reset by an opposite turn, a 180 reversal, or
    // the chain outliving SPIN_WINDOW_FRAMES.
    spin_sum: f32,
    // Frame and position at which the current chain started, for the
    // window and net-drift checks.
    spin_start_frame: u32,
    spin_start_pos: Position,
    // --- churn: trailing-window path length vs net displacement ---
    churn_flagged: bool,
    // Per-frame position samples covering the trailing CHURN_WINDOW_FRAMES,
    // oldest first, with the summed segment lengths between consecutive
    // samples maintained incrementally alongside.
    trail: VecDeque<(u32, Position)>,
    trail_path_len: f32,
    // --- deliberate-hold detection (see FIRED_RECENTLY_FRAMES) ---
    // Last frame's (shells, minigun, plasma, laser) ammo, to spot a
    // trigger pull as any pool decreasing; None until the first frame.
    prev_ammo: Option<(i32, i32, i32, i32)>,
    // Frame of the most recent detected shot, if any.
    last_fire_frame: Option<u32>,
    // --- contact metrics (wall-grind / bump-rate / low-progress) ---
    grind_frames: u32,
    grind_flagged: bool,
    // Whether the hull touched static terrain last frame, so a bump is
    // counted on the rising edge only, not per contact frame.
    was_touching: bool,
    // Frames of recent touch onsets, for the trailing BUMP_WINDOW count.
    recent_bumps: VecDeque<u32>,
    bump_flagged: bool,
    progress_frames: u32,
    progress_flagged: bool,
    // --- round-total accumulators for --json-out (probe-side only: the
    // sim deliberately reports instantaneous facts, never accumulators -
    // see TankSnapshot's doc comments) ---
    // Static-contact onsets across the whole round - `recent_bumps`'
    // unwindowed counterpart.
    total_bumps: u32,
    // Seconds spent being driven against static terrain (the same
    // condition `grind_frames` counts, summed over the round).
    grind_seconds: f32,
    // Full path length driven this round - `trail_path_len`'s unwindowed
    // counterpart.
    total_path_len: f32,
    // --- navigation e2e (path stretch / never-arrived) ---
    // Shortest-route length from this tank's spawn to the player's
    // round-start position, in nav-grid cells (`Game::nav_path_cells`) -
    // `None` means no route existed at spawn, which excludes the tank
    // from the never-arrived check entirely - and the ideal traversal
    // time that route costs at this tank's own rolled top speed. Both
    // filled once by `run_round` right after init.
    path_cells: Option<u32>,
    ideal_seconds: f32,
    // First simulated second this tank came within ENEMY_ATTACK_RANGE of
    // the player's current position - first crossing only, so a tank
    // that engaged once and then retreated (or was wrecked afterwards)
    // still counts as having arrived. `None` = never engaged.
    time_to_engage: Option<f32>,
}

impl TankTrack {
    /// True while this tank has a fighting reason to be standing still -
    /// mirrors the three ai.rs behaviors that hold position on purpose:
    /// it fired within the trailing FIRED_RECENTLY_FRAMES window
    /// (`act_attack`'s fire/cooldown rhythm); it currently holds an
    /// aligned, in-range firing solution on the live player
    /// (`act_attack`'s aim-settle hold - within ENEMY_FIRE_ALIGN_PX of a
    /// cardinal axis and inside ENEMY_ATTACK_RANGE); or it's parked
    /// outside ENEMY_RETREAT_RANGE with shells still below
    /// ENEMY_AMMO_RESUME (`act_retreat`'s wait-out-the-recharge hold).
    fn deliberate_hold(&self, frame: u32, tank: &TankSnapshot, player: &TankSnapshot) -> bool {
        if self
            .last_fire_frame
            .is_some_and(|f| frame - f <= FIRED_RECENTLY_FRAMES)
        {
            return true;
        }
        if player.is_wreck {
            return false;
        }
        let dx = (player.position.x - tank.position.x).abs();
        let dy = (player.position.y - tank.position.y).abs();
        let dist = tank.position.distance_to(player.position);
        let aligned_in_range = dx.min(dy) <= tuning().enemy_fire_align_px && dist <= tuning().enemy_attack_range;
        let retreat_wait = dist >= tuning().enemy_retreat_range() && tank.shells_ammo < tuning().enemy_ammo_resume;
        aligned_in_range || retreat_wait
    }
}

impl TankTrack {
    fn new(label: String, spawn_pos: Position, initial_rotation: f32) -> Self {
        let mut heading_history = VecDeque::with_capacity(3);
        heading_history.push_back(initial_rotation);
        Self {
            label,
            spawn_pos,
            max_spawn_dist: 0.0,
            stall_frames: 0,
            border_frames: 0,
            stale_flagged: false,
            stall_flagged: false,
            border_flagged: false,
            jitter_flagged: false,
            cluster_frames: 0,
            cluster_flagged: false,
            heading_history,
            recent_flips: VecDeque::new(),
            spin_flagged: false,
            last_heading: initial_rotation,
            spin_sum: 0.0,
            spin_start_frame: 0,
            spin_start_pos: spawn_pos,
            churn_flagged: false,
            trail: VecDeque::with_capacity(CHURN_WINDOW_FRAMES as usize + 1),
            trail_path_len: 0.0,
            prev_ammo: None,
            last_fire_frame: None,
            grind_frames: 0,
            grind_flagged: false,
            was_touching: false,
            recent_bumps: VecDeque::new(),
            bump_flagged: false,
            progress_frames: 0,
            progress_flagged: false,
            total_bumps: 0,
            grind_seconds: 0.0,
            total_path_len: 0.0,
            path_cells: None,
            ideal_seconds: 0.0,
            time_to_engage: None,
        }
    }
}

/// The signed quarter-turn from heading `old` to heading `new` (both always
/// exactly one of 0/90/180/270 - see `Dir::rotation`): `Some(90.0)` for a
/// clockwise turn, `Some(-90.0)` for counter-clockwise, `None` for a 180
/// reversal (which breaks a rotational chain rather than extending it -
/// see SPIN_WINDOW_FRAMES's comment).
fn signed_quarter_turn(old: f32, new: f32) -> Option<f32> {
    let delta = (new - old).rem_euclid(360.0);
    if (delta - 90.0).abs() < 1.0 {
        Some(90.0)
    } else if (delta - 270.0).abs() < 1.0 {
        Some(-90.0)
    } else {
        None
    }
}

/// Prints an `ANOMALY` line; the caller tallies the kind into
/// `AnomalyTotals`. `seed` is the round's own effective seed
/// (`Game::round_seed`), printed in `--seed`-pasteable form so every
/// single anomaly is a self-contained replay recipe. `heat` collects the
/// (kind, position) pair for `--heatmap`'s end-of-run render - collected
/// unconditionally (a few pairs per flagged round) so the flag needs no
/// plumbing into every check.
fn report(
    heat: &mut Vec<(String, Position)>,
    round: u32,
    seed: u64,
    frame: u32,
    label: &str,
    kind: &str,
    detail: &str,
    pos: Position,
) {
    println!(
        "ANOMALY round={round} seed=0x{seed:016x} frame={frame:5} t={:6.2}s kind={kind:<12} tank={label} pos=({:6.1},{:6.1}) {detail}",
        frame as f32 * DT,
        pos.x,
        pos.y,
    );
    heat.push((kind.to_string(), pos));
}

/// Display label for the tank at `index` of a `tank_snapshots()` slice -
/// "PLAYER", or "ENEMY#k" numbering the non-player tanks in snapshot
/// order, matching `build_tracks`/`log_frame`'s numbering exactly (the
/// snapshot order is stable for the whole round - see `TankTrack`).
fn tank_label(snapshots: &[TankSnapshot], index: usize) -> String {
    if snapshots[index].is_player {
        "PLAYER".to_string()
    } else {
        let ordinal = snapshots[..index].iter().filter(|t| !t.is_player).count();
        format!("ENEMY#{ordinal}")
    }
}

/// Runs this frame's standing checks: first the frame invariants over
/// *every* tank (player and wrecks included - see the INVARIANT_* consts),
/// then the behavior checks over enemies. Behavior checks stay scoped to
/// enemies only - the player's movement is fully explained by the scripted
/// `Scenario` already (e.g. `Afk` deliberately never moves), so checking it
/// would just be re-detecting the script, not a real anomaly. Skips checks
/// entirely once the round has ended (`Outcome != Playing`): a tank sitting
/// still on the win/lose screen isn't a bug (and post-round frames aren't
/// simulated anyway - `update` early-returns into its end-screen branch).
fn check_anomalies(
    tracks: &mut [TankTrack],
    invariant_flagged: &mut [bool],
    game: &Game,
    round: u32,
    frame: u32,
    totals: &mut AnomalyTotals,
    heat: &mut Vec<(String, Position)>,
) {
    if game.outcome() != Outcome::Playing {
        return;
    }
    let seed = game.round_seed();
    let snapshots = game.tank_snapshots();

    // --- Frame invariants: physics sanity for every tank, wrecks included
    // (a wreck keeps its physics body, so a solver blow-up can fling it
    // just the same). One-shot per tank per round, like every other kind.
    for (i, tank) in snapshots.iter().enumerate() {
        if invariant_flagged[i] {
            continue;
        }
        let pos = tank.position;
        let vel = tank.velocity;
        let finite =
            pos.x.is_finite() && pos.y.is_finite() && vel.x.is_finite() && vel.y.is_finite();
        let speed = (vel.x * vel.x + vel.y * vel.y).sqrt();
        let in_bounds = pos.x >= -INVARIANT_POS_MARGIN
            && pos.x <= WIDTH + INVARIANT_POS_MARGIN
            && pos.y >= -INVARIANT_POS_MARGIN
            && pos.y <= HEIGHT + INVARIANT_POS_MARGIN;
        // Ordered so a NaN reports as itself rather than as a bounds/speed
        // trip (every comparison involving a NaN is false).
        let violation = if !finite {
            format!("non-finite position/velocity vel=({},{})", vel.x, vel.y)
        } else if !in_bounds {
            format!("escaped battlefield bounds (margin {INVARIANT_POS_MARGIN:.0}px)")
        } else if speed > INVARIANT_SPEED_MAX {
            format!("speed {speed:.0}px/s above sanity cap {INVARIANT_SPEED_MAX:.0}px/s")
        } else {
            continue;
        };
        report(
            heat,
            round,
            seed,
            frame,
            &tank_label(&snapshots, i),
            "invariant",
            &violation,
            pos,
        );
        invariant_flagged[i] = true;
        totals.invariant += 1;
    }

    // Live (non-wreck) enemy positions, indexed in lockstep with `tracks` -
    // built up front so the clustering check below can look at every other
    // enemy's position, not just the one `TankTrack` the main loop happens
    // to be on.
    let live_positions: Vec<Option<Position>> = snapshots
        .iter()
        .filter(|t| !t.is_player)
        .map(|t| (!t.is_wreck).then_some(t.position))
        .collect();
    let player_snap = snapshots
        .iter()
        .find(|t| t.is_player)
        .expect("snapshots always include the player");
    let mut enemy_index = 0;
    for tank in &snapshots {
        if tank.is_player {
            continue;
        }
        let track = &mut tracks[enemy_index];
        let my_index = enemy_index;
        enemy_index += 1;
        if tank.is_wreck {
            continue;
        }
        let pos = tank.position;
        let speed = (tank.velocity.x * tank.velocity.x + tank.velocity.y * tank.velocity.y).sqrt();
        // The commanded counterpart (see TankSnapshot::commanded_velocity):
        // what the AI asked for this frame, vs `speed` = what physics gave.
        let cmd_speed = (tank.commanded_velocity.x * tank.commanded_velocity.x
            + tank.commanded_velocity.y * tank.commanded_velocity.y)
            .sqrt();

        // Navigation e2e: first crossing into engagement range of the
        // player's *current* position - the "arrived" instant for the
        // path-stretch metric (see NAV_GRACE_SECONDS's comment). Recorded
        // here, judged once at round end by `run_round`.
        if track.time_to_engage.is_none()
            && pos.distance_to(player_snap.position) <= tuning().enemy_attack_range
        {
            track.time_to_engage = Some(frame as f32 * DT);
        }

        // Spot trigger pulls (any ammo pool decreasing), then decide
        // whether standing still right now is a fighting hold rather than
        // a failure - see FIRED_RECENTLY_FRAMES / `deliberate_hold`.
        let ammo = (
            tank.shells_ammo,
            tank.minigun_ammo,
            tank.plasma_ammo,
            tank.laser_charges,
        );
        if let Some(prev) = track.prev_ammo
            && (ammo.0 < prev.0 || ammo.1 < prev.1 || ammo.2 < prev.2 || ammo.3 < prev.3)
        {
            track.last_fire_frame = Some(frame);
        }
        track.prev_ammo = Some(ammo);
        let holding = track.deliberate_hold(frame, tank, player_snap);

        // Stale-start: never got clear of spawn within STALE_START_FRAMES.
        if frame <= STALE_START_FRAMES {
            track.max_spawn_dist = track.max_spawn_dist.max(pos.distance_to(track.spawn_pos));
        }
        if !track.stale_flagged && frame == STALE_START_FRAMES && !holding {
            if track.max_spawn_dist < STALE_START_EPS {
                report(
                    heat,
                    round,
                    seed,
                    frame,
                    &track.label,
                    "stale-start",
                    &format!("hasn't left spawn in {STALE_START_FRAMES} frames"),
                    pos,
                );
                track.stale_flagged = true;
                totals.stale_start += 1;
            }
        }

        // Mid-round stall: near-zero speed for a sustained window, checked
        // only after the stale-start window so the two don't double-report
        // the same "never moved" tank.
        if frame > STALE_START_FRAMES {
            if speed < STALL_SPEED_EPS && !holding {
                track.stall_frames += 1;
            } else {
                track.stall_frames = 0;
            }
            if !track.stall_flagged && track.stall_frames >= STALL_FRAMES_THRESHOLD {
                report(
                    heat,
                    round,
                    seed,
                    frame,
                    &track.label,
                    "stall",
                    &format!("speed <{STALL_SPEED_EPS:.0}px/s for {STALL_FRAMES_THRESHOLD} frames"),
                    pos,
                );
                track.stall_flagged = true;
                totals.stall += 1;
            }
        }

        // Border-stuck: hugging one of the four battlefield walls.
        let dist_to_border = pos.x.min(WIDTH - pos.x).min(pos.y).min(HEIGHT - pos.y);
        if dist_to_border <= BORDER_MARGIN {
            track.border_frames += 1;
        } else {
            track.border_frames = 0;
        }
        if !track.border_flagged && track.border_frames >= BORDER_FRAMES_THRESHOLD {
            report(
                heat,
                round,
                seed,
                frame,
                &track.label,
                "border-stuck",
                &format!("within {BORDER_MARGIN:.0}px of a wall for {BORDER_FRAMES_THRESHOLD} frames"),
                pos,
            );
            track.border_flagged = true;
            totals.border_stuck += 1;
        }

        // Facing jitter: A,B,A heading flip-flops within a trailing window.
        if track.heading_history.back() != Some(&tank.rotation) {
            track.heading_history.push_back(tank.rotation);
            if track.heading_history.len() > 3 {
                track.heading_history.pop_front();
            }
            if track.heading_history.len() == 3
                && track.heading_history[0] == track.heading_history[2]
            {
                track.recent_flips.push_back(frame);
                while let Some(&oldest) = track.recent_flips.front() {
                    if frame - oldest > JITTER_WINDOW_FRAMES {
                        track.recent_flips.pop_front();
                    } else {
                        break;
                    }
                }
                if !track.jitter_flagged && track.recent_flips.len() as u32 >= JITTER_THRESHOLD {
                    report(
                        heat,
                        round,
                        seed,
                        frame,
                        &track.label,
                        "jitter",
                        &format!(
                            "{JITTER_THRESHOLD}+ heading flip-flops within {JITTER_WINDOW_FRAMES} frames"
                        ),
                        pos,
                    );
                    track.jitter_flagged = true;
                    totals.jitter += 1;
                }
            }
        }

        // Spin: chain up consecutive same-direction quarter-turns; a full
        // circle inside SPIN_WINDOW_FRAMES that ends near where it started
        // is a tank rotating in place, not navigating (see the SPIN_*
        // consts' comment for why legit routing can't complete one).
        if tank.rotation != track.last_heading {
            let turn = signed_quarter_turn(track.last_heading, tank.rotation);
            track.last_heading = tank.rotation;
            match turn {
                Some(delta) => {
                    let same_dir =
                        track.spin_sum != 0.0 && (track.spin_sum > 0.0) == (delta > 0.0);
                    let in_window = frame - track.spin_start_frame <= SPIN_WINDOW_FRAMES;
                    if same_dir && in_window {
                        track.spin_sum += delta;
                    } else {
                        track.spin_sum = delta;
                        track.spin_start_frame = frame;
                        track.spin_start_pos = pos;
                    }
                    let net_drift = pos.distance_to(track.spin_start_pos);
                    if !track.spin_flagged
                        && track.spin_sum.abs() >= SPIN_FULL_CIRCLE_DEG
                        && net_drift <= SPIN_NET_MAX
                    {
                        report(
                            heat,
                            round,
                            seed,
                            frame,
                            &track.label,
                            "spin",
                            &format!(
                                "{:.0}-degree same-direction heading rotation within {} frames (net drift {net_drift:.0}px)",
                                track.spin_sum.abs(),
                                frame - track.spin_start_frame,
                            ),
                            pos,
                        );
                        track.spin_flagged = true;
                        totals.spin += 1;
                        track.spin_sum = 0.0;
                    }
                }
                // A 180 reversal: not a rotation - break the chain.
                None => {
                    track.spin_sum = 0.0;
                    track.spin_start_frame = frame;
                    track.spin_start_pos = pos;
                }
            }
        }

        // Churn: maintain the trailing position trail and compare path
        // length against net displacement once the window is full.
        if let Some(&(_, last_pos)) = track.trail.back() {
            let segment = last_pos.distance_to(pos);
            track.trail_path_len += segment;
            track.total_path_len += segment;
        }
        track.trail.push_back((frame, pos));
        while let Some(&(oldest_frame, oldest_pos)) = track.trail.front() {
            if frame - oldest_frame > CHURN_WINDOW_FRAMES {
                track.trail.pop_front();
                if let Some(&(_, next_pos)) = track.trail.front() {
                    track.trail_path_len -= oldest_pos.distance_to(next_pos);
                }
            } else {
                break;
            }
        }
        if !track.churn_flagged
            && let Some(&(oldest_frame, oldest_pos)) = track.trail.front()
            && frame - oldest_frame >= CHURN_WINDOW_FRAMES
        {
            let net = oldest_pos.distance_to(pos);
            if track.trail_path_len >= CHURN_MIN_PATH && net <= CHURN_NET_MAX {
                report(
                    heat,
                    round,
                    seed,
                    frame,
                    &track.label,
                    "churn",
                    &format!(
                        "traveled {:.0}px but net displacement only {net:.0}px over {CHURN_WINDOW_FRAMES} frames",
                        track.trail_path_len,
                    ),
                    pos,
                );
                track.churn_flagged = true;
                totals.churn += 1;
            }
        }

        // Wall-grind: being driven while the hull holds an active static
        // contact, sustained (see the GRIND_* consts). Resets the moment
        // either half stops being true, so brushing a corner in passing
        // never accumulates across separate touches.
        if tank.touching_static && cmd_speed > GRIND_CMD_EPS {
            track.grind_frames += 1;
            track.grind_seconds += DT;
        } else {
            track.grind_frames = 0;
        }
        if !track.grind_flagged && track.grind_frames >= GRIND_FRAMES {
            report(
                heat,
                round,
                seed,
                frame,
                &track.label,
                "wall-grind",
                &format!(
                    "driving into static terrain for {GRIND_FRAMES} frames (speed {speed:.0} of commanded {cmd_speed:.0}px/s, impulse {:.0})",
                    tank.contact_impulse,
                ),
                pos,
            );
            track.grind_flagged = true;
            totals.wall_grind += 1;
        }

        // Bump-rate: count static-contact onsets (rising edges) in the
        // trailing BUMP_WINDOW_FRAMES; too many means the tank keeps
        // driving into terrain even if no single touch lasts GRIND_FRAMES.
        let bumped_now = tank.touching_static && !track.was_touching;
        if bumped_now {
            track.recent_bumps.push_back(frame);
            track.total_bumps += 1;
        }
        track.was_touching = tank.touching_static;
        while let Some(&oldest) = track.recent_bumps.front() {
            if frame - oldest > BUMP_WINDOW_FRAMES {
                track.recent_bumps.pop_front();
            } else {
                break;
            }
        }
        if !track.bump_flagged && track.recent_bumps.len() as u32 > BUMP_RATE_MAX {
            report(
                heat,
                round,
                seed,
                frame,
                &track.label,
                "bump-rate",
                &format!(
                    "{} static-terrain bumps within {BUMP_WINDOW_FRAMES} frames (cap {BUMP_RATE_MAX})",
                    track.recent_bumps.len(),
                ),
                pos,
            );
            track.bump_flagged = true;
            totals.bump_rate += 1;
        }

        // Low-progress: commanded a real fraction of top speed, achieving
        // under a third of it, sustained (see the PROGRESS_* consts).
        if cmd_speed >= PROGRESS_CMD_FRACTION * tank.top_speed
            && speed < PROGRESS_ACHIEVED_FRACTION * cmd_speed
        {
            track.progress_frames += 1;
        } else {
            track.progress_frames = 0;
        }
        if !track.progress_flagged && track.progress_frames >= PROGRESS_FRAMES {
            report(
                heat,
                round,
                seed,
                frame,
                &track.label,
                "low-progress",
                &format!(
                    "achieving {speed:.0}px/s of a commanded {cmd_speed:.0}px/s for {PROGRESS_FRAMES} frames"
                ),
                pos,
            );
            track.progress_flagged = true;
            totals.low_progress += 1;
        }

        // Clustering: how many other live enemies are mutually within
        // CLUSTER_RADIUS of this one right now.
        let nearby = live_positions
            .iter()
            .enumerate()
            .filter(|&(i, other)| {
                i != my_index && other.is_some_and(|p| p.distance_to(pos) <= CLUSTER_RADIUS)
            })
            .count();
        if nearby + 1 >= CLUSTER_MIN_GROUP {
            track.cluster_frames += 1;
        } else {
            track.cluster_frames = 0;
        }
        if !track.cluster_flagged && track.cluster_frames >= CLUSTER_FRAMES_THRESHOLD {
            report(
                heat,
                round,
                seed,
                frame,
                &track.label,
                "clustering",
                &format!(
                    "{CLUSTER_MIN_GROUP}+ enemies mutually within {CLUSTER_RADIUS:.0}px for {CLUSTER_FRAMES_THRESHOLD} frames"
                ),
                pos,
            );
            track.cluster_flagged = true;
            totals.clustering += 1;
        }
    }
}

fn build_tracks(game: &Game) -> Vec<TankTrack> {
    let mut enemy_index = 0;
    game.tank_snapshots()
        .into_iter()
        .filter(|t| !t.is_player)
        .map(|t| {
            let label = format!("ENEMY#{enemy_index}");
            enemy_index += 1;
            TankTrack::new(label, t.position, t.rotation)
        })
        .collect()
}

/// Per-enemy end-of-round metrics for one `--json-out` record, read off
/// the same `TankTrack` state the anomaly checks maintained all round.
struct TankReport {
    label: String,
    time_to_engage: Option<f32>,
    /// `time_to_engage / ideal_seconds` - `None` when there was no route,
    /// no engagement, or the tank spawned essentially in range (ideal ~ 0,
    /// where a ratio is noise) - the same rule the trace table prints.
    stretch: Option<f32>,
    path_cells: Option<u32>,
    contact_events: u32,
    grind_seconds: f32,
    distance_travelled: f32,
}

/// Everything one round produces, for `main` to fold into the sweep
/// totals, the `--json-out` record, and the `--budget` verdict.
struct RoundResult {
    totals: AnomalyTotals,
    frames_run: u32,
    outcome: Outcome,
    tanks: Vec<TankReport>,
}

/// Runs one round to completion (or the frame limit). `trace` controls
/// whether the per-frame `log_frame` snapshots print (single-round mode)
/// or stay silent (sweep mode, where only ANOMALY lines and the round
/// summary matter). `seed` pins the round's whole RNG stream
/// (`Game::seed_override`), derived per round by `main` - see the `--seed`
/// flag's doc comment. `heat` collects every reported anomaly's (kind,
/// position) for `--heatmap`.
fn run_round(
    args: &Args,
    round: u32,
    trace: bool,
    seed: u64,
    heat: &mut Vec<(String, Position)>,
) -> RoundResult {
    let mut game = Game::default();
    game.enemy_count_override = args.enemies;
    game.level_overrides = bongbong::level::LevelOverrides {
        mission: args.mission,
        spawn: args.spawn,
        waves: args.waves,
        wave_size: args.wave_size,
        wave_growth: args.wave_growth,
        tier_start: args.tier_start,
        tier_end: args.tier_end,
    };
    game.seed_override = Some(seed);
    game.map = match &args.map {
        Some(named) => named.map.clone(),
        None => default_map(),
    };
    game.init(WIDTH, HEIGHT);

    let mut tracks = build_tracks(&game);
    // One flag slot per tank (player included, unlike `tracks`) for the
    // one-shot frame-invariant checks - same stable snapshot-order
    // indexing convention as `TankTrack`.
    let mut invariant_flagged = vec![false; game.tank_snapshots().len()];
    let mut totals = AnomalyTotals::default();

    // Frame-0 navigation baseline for the path-stretch metric (see
    // NAV_GRACE_SECONDS's comment): each enemy's shortest-route length to
    // where the player starts, and the ideal seconds that route costs at
    // the tank's own rolled top speed. Once per round - `nav_path_cells`
    // builds a fresh grid per call, cheap here, waste per frame.
    {
        let snapshots = game.tank_snapshots();
        let player_pos = snapshots
            .iter()
            .find(|t| t.is_player)
            .expect("snapshots always include the player")
            .position;
        let mut enemy_index = 0;
        for tank in &snapshots {
            if tank.is_player {
                continue;
            }
            let track = &mut tracks[enemy_index];
            enemy_index += 1;
            track.path_cells = game.nav_path_cells(tank.position, player_pos, WIDTH, HEIGHT);
            if let Some(cells) = track.path_cells {
                // top_speed is a real rolled speed (150-220px/s range),
                // never zero - the .max(1.0) only guards a hypothetical
                // future zero-speed chassis from poisoning the metric
                // with an infinity.
                track.ideal_seconds = cells as f32 * PATHFIND_CELL_SIZE / tank.top_speed.max(1.0);
            }
        }
    }

    if trace {
        log_frame(&game, 0);
    }
    check_anomalies(&mut tracks, &mut invariant_flagged, &game, round, 0, &mut totals, heat);

    let mut frames_run = args.frames;
    for frame in 1..=args.frames {
        let input = input_for_frame(args.scenario, frame);
        game.update(input, DT, WIDTH, HEIGHT);
        check_anomalies(&mut tracks, &mut invariant_flagged, &game, round, frame, &mut totals, heat);

        if trace && frame % args.log_every == 0 {
            log_frame(&game, frame);
        }

        if game.outcome() != Outcome::Playing {
            frames_run = frame;
            if trace {
                log_frame(&game, frame);
                println!(
                    "probe: round ended after {frame} frames ({:.2}s)",
                    frame as f32 * DT
                );
            }
            break;
        }
    }

    // Round over (or frame cap): the one-shot navigation verdict, outside
    // check_anomalies' still-Playing gate on purpose - "never arrived" is
    // only decidable once no more arriving can happen. The per-enemy
    // stretch table prints in trace mode either way: the distribution is
    // the real navigation-health signal, the anomaly just its tail.
    let elapsed = frames_run as f32 * DT;
    let snapshots = game.tank_snapshots();
    let mut reports: Vec<TankReport> = Vec::with_capacity(tracks.len());
    let mut enemy_index = 0;
    for tank in &snapshots {
        if tank.is_player {
            continue;
        }
        let track = &tracks[enemy_index];
        enemy_index += 1;
        reports.push(TankReport {
            label: track.label.clone(),
            time_to_engage: track.time_to_engage,
            stretch: match (track.path_cells, track.time_to_engage) {
                (Some(_), Some(t)) if track.ideal_seconds > 0.05 => {
                    Some(t / track.ideal_seconds)
                }
                _ => None,
            },
            path_cells: track.path_cells,
            contact_events: track.total_bumps,
            grind_seconds: track.grind_seconds,
            distance_travelled: track.total_path_len,
        });
        if trace {
            let nav = match (track.path_cells, track.time_to_engage) {
                (None, _) => "route=none (no path at spawn)".to_string(),
                (Some(cells), Some(t)) => format!(
                    "route={cells}c ideal={:.1}s engaged={t:.1}s stretch={}",
                    track.ideal_seconds,
                    if track.ideal_seconds > 0.05 {
                        format!("{:.1}", t / track.ideal_seconds)
                    } else {
                        "-".to_string() // spawned essentially in range
                    },
                ),
                (Some(cells), None) => format!(
                    "route={cells}c ideal={:.1}s engaged=never (budget {:.1}s, elapsed {elapsed:.1}s)",
                    track.ideal_seconds,
                    NAV_GRACE_SECONDS + NAV_STRETCH_MAX * track.ideal_seconds,
                ),
            };
            println!("probe: nav {} {nav}", track.label);
        }
        let (Some(cells), None) = (track.path_cells, track.time_to_engage) else {
            continue;
        };
        let budget = NAV_GRACE_SECONDS + NAV_STRETCH_MAX * track.ideal_seconds;
        if !tank.is_wreck && elapsed > budget {
            report(
                heat,
                round,
                game.round_seed(),
                frames_run,
                &track.label,
                "never-arrived",
                &format!(
                    "alive, route existed ({cells} cells, ideal {:.1}s) but no engagement in {elapsed:.1}s (budget {budget:.1}s)",
                    track.ideal_seconds,
                ),
                tank.position,
            );
            totals.never_arrived += 1;
        }
    }

    RoundResult { totals, frames_run, outcome: game.outcome(), tanks: reports }
}

/// The header/JSONL name for the scenario - one place, so the two can't
/// disagree.
fn scenario_str(scenario: Scenario) -> &'static str {
    match scenario {
        Scenario::Afk => "afk",
        Scenario::Advance => "advance",
        Scenario::Brake => "brake",
        Scenario::Circle => "circle",
    }
}

/// The battlefield's display name: the `--map` path, or the embedded
/// default's marker - shared by the header, `--json-out`, and `--heatmap`.
fn map_display(args: &Args) -> &str {
    args.map
        .as_ref()
        .map(|m| m.name.as_str())
        .unwrap_or("maps/default.toml (embedded)")
}

/// Minimal JSON string escaping for the few strings the JSONL schema
/// carries (map path, scenario, outcome, tank label): backslash, quote,
/// and control characters - everything else passes through as UTF-8,
/// which JSON allows raw.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// One `--json-out` line: the round's whole record as a single JSON
/// object (see the flag's doc comment for the schema contract; "v":1).
/// Hand-emitted - the schema is flat numbers plus four short strings, not
/// worth a serde_json dependency (same call the map format made for TOML
/// only because serde was already there for it).
fn json_round_line(args: &Args, round: u32, seed: u64, result: &RoundResult, tuning_diff: &str) -> String {
    let anomalies = ANOMALY_KINDS
        .iter()
        .map(|kind| format!("\"{}\":{}", kind.replace('-', "_"), result.totals.count(kind)))
        .collect::<Vec<_>>()
        .join(",");
    let opt_f32 = |v: Option<f32>| v.map_or("null".to_string(), |v| format!("{v:.2}"));
    let tanks = result
        .tanks
        .iter()
        .map(|t| {
            format!(
                "{{\"label\":\"{}\",\"time_to_engage\":{},\"stretch\":{},\"path_cells\":{},\"contact_events\":{},\"grind_seconds\":{:.2},\"distance_travelled\":{:.1}}}",
                json_escape(&t.label),
                opt_f32(t.time_to_engage),
                opt_f32(t.stretch),
                t.path_cells.map_or("null".to_string(), |v| v.to_string()),
                t.contact_events,
                t.grind_seconds,
                t.distance_travelled,
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"v\":1,\"round\":{round},\"seed\":\"0x{seed:016x}\",\"tuning\":{tuning_diff},\"map\":\"{}\",\"scenario\":\"{}\",\"enemies\":{},\"frames_run\":{},\"outcome\":\"{}\",\"anomalies\":{{{anomalies}}},\"tanks\":[{tanks}]}}",
        json_escape(map_display(args)),
        scenario_str(args.scenario),
        result.tanks.len(),
        result.frames_run,
        outcome_str(result.outcome),
    )
}

/// `--heatmap`: for each kind that fired, an ASCII battlefield at
/// PATHFIND_CELL_SIZE granularity (the same nav grid the AI routes on, so
/// a hot cell here is directly a nav-grid cell to stare at) - `·` none,
/// `1`-`9` that many hits, `#` ten or more. Positions are clamped into
/// the grid (an `invariant` escapee can sit outside it).
fn print_heatmaps(args: &Args, heat: &[(String, Position)]) {
    if heat.is_empty() {
        println!("probe: heatmap: no anomalies to map");
        return;
    }
    let cols = (WIDTH / PATHFIND_CELL_SIZE).ceil() as usize;
    let rows = (HEIGHT / PATHFIND_CELL_SIZE).ceil() as usize;
    for kind in ANOMALY_KINDS {
        let hits: Vec<Position> =
            heat.iter().filter(|(k, _)| k == kind).map(|&(_, p)| p).collect();
        if hits.is_empty() {
            continue;
        }
        let mut grid = vec![0u32; cols * rows];
        for p in &hits {
            let col = ((p.x / PATHFIND_CELL_SIZE) as isize).clamp(0, cols as isize - 1) as usize;
            let row = ((p.y / PATHFIND_CELL_SIZE) as isize).clamp(0, rows as isize - 1) as usize;
            grid[row * cols + col] += 1;
        }
        println!("probe: heatmap kind={kind} hits={} map={}", hits.len(), map_display(args));
        for row in 0..rows {
            let line: String = (0..cols)
                .map(|col| match grid[row * cols + col] {
                    0 => '\u{b7}', // ·
                    n @ 1..=9 => char::from_digit(n, 10).expect("1-9 is a digit"),
                    _ => '#',
                })
                .collect();
            println!("  {line}");
        }
    }
    println!(
        "probe: heatmap scale: one cell = {PATHFIND_CELL_SIZE:.0}x{PATHFIND_CELL_SIZE:.0}px of battlefield, '\u{b7}'=0 '1'-'9'=hits '#'=10+"
    );
}

/// `--budget` verdict against the whole run's totals: prints one
/// `BUDGET EXCEEDED` line per breach and reports whether any fired - the
/// caller turns that into exit code 1. Runs after every other output so a
/// CI log shows the full sweep (and heatmaps) above the verdict.
fn check_budgets(budgets: &[Budget], totals: &AnomalyTotals) -> bool {
    let mut exceeded = false;
    for budget in budgets {
        let count = if budget.kind == "total" {
            totals.total()
        } else {
            totals.count(&budget.kind)
        };
        if count > budget.max {
            println!("BUDGET EXCEEDED kind={} count={count} max={}", budget.kind, budget.max);
            exceeded = true;
        }
    }
    exceeded
}

fn main() -> ExitCode {
    let args = Args::parse();
    let sweep = args.rounds > 1;

    // The whole run's base seed: `--seed` when given, one entropy draw
    // otherwise - either way it's printed in the header, so *every* run is
    // replayable after the fact. Round i runs on base+i (see the `--seed`
    // flag's doc comment; SmallRng's seed_from_u64 mixes the raw value
    // through SplitMix64, so adjacent seeds are fully decorrelated rounds).
    let base_seed: u64 = args.seed.unwrap_or_else(|| rand::rng().random());

    // `--tuning`: applied once, before any round, straight into the live
    // table (there's no frame loop to stage against yet). The diff is part
    // of the replay recipe alongside the seed, so it's echoed in the header
    // and in every --json-out record.
    if let Some(path) = &args.tuning {
        if let Err(e) = bongbong::tuning::submit_file(path) {
            eprintln!("--tuning: {e}");
            return ExitCode::from(2);
        }
        bongbong::tuning::apply_pending();
    }
    let tuning_diff = bongbong::tuning::diff_json();
    if args.print_rust {
        print!("{}", bongbong::tuning::diff_rust());
        return ExitCode::SUCCESS;
    }

    // Created (truncating) up front so a bad path fails before any rounds
    // burn time; each round's record is written as it finishes, so even an
    // interrupted sweep leaves valid JSONL behind.
    let mut json_out = args.json_out.as_ref().map(|path| {
        File::create(path).unwrap_or_else(|e| {
            panic!("--json-out {}: {e}", path.display());
        })
    });

    println!(
        "probe: scenario={} enemies={} frames={} rounds={} seed=0x{base_seed:016x} map={} tuning={}",
        scenario_str(args.scenario),
        args.enemies
            .map(|n| n.to_string())
            .unwrap_or_else(|| "random".to_string()),
        args.frames,
        args.rounds,
        map_display(&args),
        if tuning_diff == "{}" { "default".to_string() } else { tuning_diff.clone() },
    );

    let mut grand_total = AnomalyTotals::default();
    // Every flagged round's (round, seed), for the end-of-sweep replay
    // recap - the per-round lines above it scroll away on a big sweep.
    let mut flagged: Vec<(u32, u64)> = Vec::new();
    // Every reported anomaly's (kind, position), for --heatmap.
    let mut heat: Vec<(String, Position)> = Vec::new();

    for round in 0..args.rounds {
        let seed = base_seed.wrapping_add(round as u64);
        let result = run_round(&args, round, !sweep, seed, &mut heat);
        if let Some(file) = &mut json_out {
            writeln!(file, "{}", json_round_line(&args, round, seed, &result, &tuning_diff))
                .unwrap_or_else(|e| panic!("writing --json-out: {e}"));
        }
        let totals = result.totals;
        if sweep {
            if totals.total() > 0 {
                flagged.push((round, seed));
                println!(
                    "round={round} seed=0x{seed:016x} anomalies={} (stale-start={} stall={} border-stuck={} jitter={} spin={} churn={} clustering={} wall-grind={} bump-rate={} low-progress={} never-arrived={} invariant={})",
                    totals.total(),
                    totals.stale_start,
                    totals.stall,
                    totals.border_stuck,
                    totals.jitter,
                    totals.spin,
                    totals.churn,
                    totals.clustering,
                    totals.wall_grind,
                    totals.bump_rate,
                    totals.low_progress,
                    totals.never_arrived,
                    totals.invariant,
                );
            }
            grand_total.stale_start += totals.stale_start;
            grand_total.stall += totals.stall;
            grand_total.border_stuck += totals.border_stuck;
            grand_total.jitter += totals.jitter;
            grand_total.spin += totals.spin;
            grand_total.churn += totals.churn;
            grand_total.clustering += totals.clustering;
            grand_total.wall_grind += totals.wall_grind;
            grand_total.bump_rate += totals.bump_rate;
            grand_total.low_progress += totals.low_progress;
            grand_total.never_arrived += totals.never_arrived;
            grand_total.invariant += totals.invariant;
        } else {
            grand_total = totals;
        }
    }

    if sweep {
        println!(
            "probe: {}/{} rounds flagged - totals: stale-start={} stall={} border-stuck={} jitter={} spin={} churn={} clustering={} wall-grind={} bump-rate={} low-progress={} never-arrived={} invariant={}",
            flagged.len(),
            args.rounds,
            grand_total.stale_start,
            grand_total.stall,
            grand_total.border_stuck,
            grand_total.jitter,
            grand_total.spin,
            grand_total.churn,
            grand_total.clustering,
            grand_total.wall_grind,
            grand_total.bump_rate,
            grand_total.low_progress,
            grand_total.never_arrived,
            grand_total.invariant,
        );
        if !flagged.is_empty() {
            let recap = flagged
                .iter()
                .map(|&(round, seed)| format!("round {round} -> --seed 0x{seed:016x}"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("probe: replay a flagged round (with --rounds 1): {recap}");
        }
    } else if grand_total.total() == 0 {
        println!("probe: no anomalies detected");
    }

    if args.heatmap {
        print_heatmaps(&args, &heat);
    }
    if check_budgets(&args.budgets, &grand_total) {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
