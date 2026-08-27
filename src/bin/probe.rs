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

use std::collections::VecDeque;

use bongbong::Position;
use bongbong::ai::Intent;
use bongbong::map::MapFile;
use bongbong::simulation::{Game, Input, Outcome, TankSnapshot};
use bongbong::tank::Dir;
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

const WIDTH: f32 = 1280.0;
const HEIGHT: f32 = 720.0;
const DT: f32 = 1.0 / 60.0;
/// See `Scenario::Brake`'s doc comment for why this is short.
const BRAKE_HOLD_FRAMES: u32 = 18;

// --- Anomaly-detection tuning ---
// A tank must move at least this far from its spawn point within
// STALE_START_FRAMES or it's flagged as never having left spawn.
const STALE_START_FRAMES: u32 = 120; // 2s
const STALE_START_EPS: f32 = 5.0; // px
// After the stale-start window, a tank sitting near-zero speed for this many
// consecutive frames (while the round is still Playing and it isn't a
// wreck) is flagged as stalled out mid-round.
const STALL_SPEED_EPS: f32 = 5.0; // px/s - top speeds are 150-220 (TANK_SPEED/ENEMY_SPEED in lib.rs)
const STALL_FRAMES_THRESHOLD: u32 = 180; // 3s
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
const CLUSTER_RADIUS: f32 = 90.0; // px between tank centers
const CLUSTER_MIN_GROUP: usize = 3; // this many mutually-close enemies counts as a cluster
const CLUSTER_FRAMES_THRESHOLD: u32 = 180; // 3s, matching STALL_FRAMES_THRESHOLD's window
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
    }
    Input {
        player_intent,
        pause_pressed: false,
        restart_pressed: false,
        toggle_shadows_pressed: false,
        toggle_inspect_pressed: false,
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
            "  {label} pos=({:6.1},{:6.1}) vel=({:6.1},{:6.1}) speed={:6.1} rot={:5.0} dmg={:5.1}/100 ammo={:2} plasma={:2} minigun={:3} wreck={}",
            tank.position.x, tank.position.y, tank.velocity.x, tank.velocity.y, speed, tank.rotation, tank.damage, tank.shells_ammo, tank.plasma_ammo, tank.minigun_ammo, tank.is_wreck,
        );
    }
}

#[derive(Default, Clone, Copy)]
struct AnomalyTotals {
    stale_start: u32,
    stall: u32,
    border_stuck: u32,
    jitter: u32,
    clustering: u32,
    invariant: u32,
}

impl AnomalyTotals {
    fn total(&self) -> u32 {
        self.stale_start
            + self.stall
            + self.border_stuck
            + self.jitter
            + self.clustering
            + self.invariant
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
}

impl TankTrack {
    fn new(label: String, spawn_pos: Position, initial_rotation: f32) -> Self {
        let mut heading_history = VecDeque::with_capacity(3);
        heading_history.push_back(initial_rotation);
        Self {
            label,
            spawn_pos,
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
        }
    }
}

/// Prints an `ANOMALY` line; the caller tallies the kind into
/// `AnomalyTotals`. `seed` is the round's own effective seed
/// (`Game::round_seed`), printed in `--seed`-pasteable form so every
/// single anomaly is a self-contained replay recipe.
fn report(round: u32, seed: u64, frame: u32, label: &str, kind: &str, detail: &str, pos: Position) {
    println!(
        "ANOMALY round={round} seed=0x{seed:016x} frame={frame:5} t={:6.2}s kind={kind:<12} tank={label} pos=({:6.1},{:6.1}) {detail}",
        frame as f32 * DT,
        pos.x,
        pos.y,
    );
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

        // Stale-start: hasn't left spawn within STALE_START_FRAMES.
        if !track.stale_flagged && frame == STALE_START_FRAMES {
            let dx = pos.x - track.spawn_pos.x;
            let dy = pos.y - track.spawn_pos.y;
            if (dx * dx + dy * dy).sqrt() < STALE_START_EPS {
                report(
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
            if speed < STALL_SPEED_EPS {
                track.stall_frames += 1;
            } else {
                track.stall_frames = 0;
            }
            if !track.stall_flagged && track.stall_frames >= STALL_FRAMES_THRESHOLD {
                report(
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

/// Runs one round to completion (or the frame limit). Returns this round's
/// anomaly totals. `trace` controls whether the per-frame `log_frame`
/// snapshots print (single-round mode) or stay silent (sweep mode, where
/// only ANOMALY lines and the round summary matter). `seed` pins the
/// round's whole RNG stream (`Game::seed_override`), derived per round by
/// `main` - see the `--seed` flag's doc comment.
fn run_round(args: &Args, round: u32, trace: bool, seed: u64) -> AnomalyTotals {
    let mut game = Game::default();
    game.enemy_count_override = args.enemies;
    game.seed_override = Some(seed);
    game.map = default_map();
    game.init(WIDTH, HEIGHT);

    let mut tracks = build_tracks(&game);
    // One flag slot per tank (player included, unlike `tracks`) for the
    // one-shot frame-invariant checks - same stable snapshot-order
    // indexing convention as `TankTrack`.
    let mut invariant_flagged = vec![false; game.tank_snapshots().len()];
    let mut totals = AnomalyTotals::default();

    if trace {
        log_frame(&game, 0);
    }
    check_anomalies(&mut tracks, &mut invariant_flagged, &game, round, 0, &mut totals);

    for frame in 1..=args.frames {
        let input = input_for_frame(args.scenario, frame);
        game.update(input, DT, WIDTH, HEIGHT);
        check_anomalies(&mut tracks, &mut invariant_flagged, &game, round, frame, &mut totals);

        if trace && frame % args.log_every == 0 {
            log_frame(&game, frame);
        }

        if game.outcome() != Outcome::Playing {
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

    totals
}

fn main() {
    let args = Args::parse();
    let sweep = args.rounds > 1;

    // The whole run's base seed: `--seed` when given, one entropy draw
    // otherwise - either way it's printed in the header, so *every* run is
    // replayable after the fact. Round i runs on base+i (see the `--seed`
    // flag's doc comment; SmallRng's seed_from_u64 mixes the raw value
    // through SplitMix64, so adjacent seeds are fully decorrelated rounds).
    let base_seed: u64 = args.seed.unwrap_or_else(|| rand::rng().random());

    println!(
        "probe: scenario={} enemies={} frames={} rounds={} seed=0x{base_seed:016x}",
        match args.scenario {
            Scenario::Afk => "afk",
            Scenario::Advance => "advance",
            Scenario::Brake => "brake",
        },
        args.enemies
            .map(|n| n.to_string())
            .unwrap_or_else(|| "random".to_string()),
        args.frames,
        args.rounds,
    );

    let mut grand_total = AnomalyTotals::default();
    // Every flagged round's (round, seed), for the end-of-sweep replay
    // recap - the per-round lines above it scroll away on a big sweep.
    let mut flagged: Vec<(u32, u64)> = Vec::new();

    for round in 0..args.rounds {
        let seed = base_seed.wrapping_add(round as u64);
        let totals = run_round(&args, round, !sweep, seed);
        if sweep {
            if totals.total() > 0 {
                flagged.push((round, seed));
                println!(
                    "round={round} seed=0x{seed:016x} anomalies={} (stale-start={} stall={} border-stuck={} jitter={} clustering={} invariant={})",
                    totals.total(),
                    totals.stale_start,
                    totals.stall,
                    totals.border_stuck,
                    totals.jitter,
                    totals.clustering,
                    totals.invariant,
                );
            }
            grand_total.stale_start += totals.stale_start;
            grand_total.stall += totals.stall;
            grand_total.border_stuck += totals.border_stuck;
            grand_total.jitter += totals.jitter;
            grand_total.clustering += totals.clustering;
            grand_total.invariant += totals.invariant;
        } else {
            grand_total = totals;
        }
    }

    if sweep {
        println!(
            "probe: {}/{} rounds flagged - totals: stale-start={} stall={} border-stuck={} jitter={} clustering={} invariant={}",
            flagged.len(),
            args.rounds,
            grand_total.stale_start,
            grand_total.stall,
            grand_total.border_stuck,
            grand_total.jitter,
            grand_total.clustering,
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
}
