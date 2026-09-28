//! `netlab run` and `netlab suite` (netlab/README.md).

use std::path::PathBuf;
use std::process::ExitCode;

use bongbong::level::Mission;
use bongbong::tank::TankKind;
use clap::{Parser, Subcommand, ValueEnum};

use netlab::link::Impairment;
use netlab::report;
use netlab::run::{self, RunConfig};
use netlab::script::Scenario;
use netlab::suite;

#[derive(Parser)]
#[command(name = "netlab", about = "How far networked play is from local play, measured")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// One run: a link, a scenario, a hull mode.
    Run(RunArgs),
    /// Profiles x scenarios x hull modes, as one markdown table.
    Suite(SuiteArgs),
    /// Measure a run's `--frames-out` dump again, offline.
    Replay(ReplayArgs),
}

#[derive(clap::Args)]
struct ReplayArgs {
    /// A dump `netlab run --frames-out` wrote.
    frames: PathBuf,
    /// List every appearance of the host's own shots online: the frame, the
    /// drawing, the shot the ledger put it down to, what it was and why.
    #[arg(long)]
    explain: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum OnOff {
    On,
    Off,
}

/// The dials every run takes, shared by `run` and passed through by
/// `suite`.
#[derive(clap::Args, Clone)]
struct Common {
    /// Frames drawn per second by each client (and the twin).
    #[arg(long, default_value_t = 60.0)]
    fps: f64,
    /// The map played, before netlab pins its enemies, spawn plan and
    /// chassis.
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/maps/arena.toml"))]
    map: PathBuf,
    /// The round's seed, decimal or 0x-hex.
    #[arg(long, default_value = "0xB0B5", value_parser = parse_seed)]
    seed: u64,
    /// Both seats' chassis. A single barrel, so one press is one `Fired`.
    #[arg(long, value_enum, default_value = "scout")]
    tank: TankKind,
    #[arg(long, value_enum, default_value = "destroy")]
    mission: Mission,
    /// Dial this room server (e.g. wss://rooms.bongbong.io/pr-48) instead
    /// of the in-process one; the tap's metrics are then absent.
    #[arg(long)]
    remote: Option<String>,
}

#[derive(clap::Args)]
struct RunArgs {
    /// lan | good | typical | mobile | bad | custom
    #[arg(long, default_value = "lan")]
    profile: String,
    #[arg(long)]
    delay_ms: Option<f64>,
    #[arg(long)]
    jitter_ms: Option<f64>,
    #[arg(long)]
    loss: Option<f64>,
    #[arg(long)]
    rto_ms: Option<f64>,
    /// Model Nagle's algorithm on both senders.
    #[arg(long)]
    nagle: bool,
    #[arg(long, default_value_t = 20.0)]
    seconds: f64,
    #[arg(long, value_enum, default_value = "drive")]
    scenario: Scenario,
    #[arg(long, value_enum, default_value = "on")]
    client_hull: OnOff,
    /// Enemies on the field; the scenario's own count when absent.
    #[arg(long)]
    enemies: Option<usize>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    /// Write every recorded frame - each seat's and the twin's: the hulls,
    /// the shots, the events and the client's readings - as JSON.
    #[arg(long)]
    frames_out: Option<PathBuf>,
    #[arg(long, default_value = "run")]
    label: String,
    /// Print nothing but errors.
    #[arg(long)]
    quiet: bool,
    #[command(flatten)]
    common: Common,
}

#[derive(clap::Args)]
struct SuiteArgs {
    /// Three profiles, eight seconds a run.
    #[arg(long)]
    quick: bool,
    /// Seconds per run, over the plan's own.
    #[arg(long)]
    seconds: Option<f64>,
    /// Every report, as one JSON array.
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[command(flatten)]
    common: Common,
}

fn parse_seed(s: &str) -> Result<u64, String> {
    let s = s.trim();
    match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).map_err(|e| e.to_string()),
        None => s.parse().map_err(|e: std::num::ParseIntError| e.to_string()),
    }
}

fn link_of(args: &RunArgs) -> Result<Impairment, String> {
    let mut link = match args.profile.as_str() {
        "custom" => Impairment::NONE,
        name => run::profile(name).ok_or_else(|| format!("unknown profile {name:?}: lan|good|typical|mobile|bad|custom"))?,
    };
    if let Some(d) = args.delay_ms {
        link.delay_ms = d;
        link.rto_ms = Impairment::default_rto_ms(d);
    }
    if let Some(j) = args.jitter_ms {
        link.jitter_ms = j;
    }
    if let Some(l) = args.loss {
        link.loss = l.clamp(0.0, 1.0);
    }
    if let Some(r) = args.rto_ms {
        link.rto_ms = r;
    }
    link.nagle |= args.nagle;
    Ok(link)
}

fn run_cmd(args: RunArgs) -> Result<(), String> {
    let cfg = RunConfig {
        label: args.label.clone(),
        profile: args.profile.clone(),
        link: link_of(&args)?,
        fps: args.common.fps,
        seconds: args.seconds,
        scenario: args.scenario,
        client_hull: args.client_hull == OnOff::On,
        map: args.common.map.clone(),
        enemies: args.enemies.unwrap_or(args.scenario.default_enemies()),
        seed: args.common.seed,
        tank: args.common.tank,
        mission: args.common.mission,
        remote: args.common.remote.clone(),
        frames_out: args.frames_out.clone(),
    };
    let report = run::run(&cfg)?;
    if !args.quiet {
        print!("{}", report::summary(&report));
    }
    if let Some(path) = &args.json_out {
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if report.online.is_none() {
        return Err(format!("the run recorded no online frames: {}", report.errors.join("; ")));
    }
    Ok(())
}

fn suite_cmd(args: SuiteArgs) -> Result<(), String> {
    let mut plan = if args.quick { suite::Plan::quick() } else { suite::Plan::full() };
    if let Some(s) = args.seconds {
        plan.seconds = s;
    }
    let c = &args.common;
    let mut extra = vec![
        "--fps".to_string(),
        c.fps.to_string(),
        "--map".into(),
        c.map.display().to_string(),
        "--seed".into(),
        c.seed.to_string(),
        "--tank".into(),
        c.tank.name().to_string(),
        "--mission".into(),
        format!("{:?}", c.mission).to_lowercase(),
    ];
    if let Some(remote) = &c.remote {
        extra.extend(["--remote".to_string(), remote.clone()]);
    }
    let scratch = std::env::temp_dir().join(format!("netlab-suite-{}", std::process::id()));
    let reports = suite::run(&plan, &extra, &scratch);
    println!("{}", suite::table(&reports));
    if let Some(path) = &args.json_out {
        let ok: Vec<&report::Report> = reports.iter().filter_map(|r| r.as_ref().ok()).collect();
        let text = serde_json::to_string_pretty(&ok).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

fn replay_cmd(args: ReplayArgs) -> Result<(), String> {
    let text = std::fs::read_to_string(&args.frames).map_err(|e| format!("{}: {e}", args.frames.display()))?;
    let dump: run::FrameDump = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", args.frames.display()))?;
    let (online, twin) = run::measure_dump(&dump);
    if let Some(online) = &online {
        print!("{}", report::metrics_lines("online", online));
    }
    print!("{}", report::metrics_lines("twin", &twin));
    if args.explain {
        println!("  the host's own shots online:");
        print!("{}", report::explain(&dump.host, &dump.guest, dump.host_seat, dump.wire.as_ref()));
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Run(args) => run_cmd(args),
        Cmd::Suite(args) => suite_cmd(args),
        Cmd::Replay(args) => replay_cmd(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("netlab: {e}");
            ExitCode::FAILURE
        }
    }
}
