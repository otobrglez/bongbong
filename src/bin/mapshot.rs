//! `mapshot`: render bongbong maps to PNG thumbnails (docs/mapshot-prd.md).
//!
//! ```text
//! mapshot maps/default.toml -o default.png
//! mapshot --out-dir target/thumbnails maps
//! mapshot --renderer gpu --scale 2 maps/missions/*.toml
//! ```
//!
//! Each PNG is the map's field - its `size` in 32 px cells, no HUD bar -
//! as a fresh round shows it on its first frame: ground, grass, walls,
//! props, trees, frogs, pickups and the player start tank(s); no enemies,
//! no banner, no locate cue. Two renderers behind `--renderer`:
//!
//! - `cpu` (the default) paints on `canvas::CpuCanvas`: no window, no GL,
//!   so it runs on a server with no display. raylib only decodes the
//!   sheets and encodes the PNG.
//! - `gpu` opens a hidden raylib window and runs the game's own pass-1
//!   stages into a render texture - the cross-check that the CPU output
//!   is the game's.
//!
//! Sheets resolve against the working directory (`static/...`), the
//! contract every bin has, so run it from the repository root or an
//! extracted release. The library side is `thumbnail.rs`.

use bongbong::map::MapFile;
use bongbong::math::Color;
use bongbong::simulation::PlayerCount;
use bongbong::tank::TankKind;
use bongbong::canvas::{Pixels, Sheet};
use bongbong::render::thumbnail::{image_png_bytes, load_cpu_sheets, render_gpu, GpuSheets};
use bongbong::thumbnail::{self, Comparison, ThumbnailOptions};
use clap::{Parser, ValueEnum};
use sola_raylib::consts::TraceLogLevel;
use sola_raylib::prelude::{RaylibHandle, RaylibThread};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
enum Renderer {
    /// The software rasteriser: no window, runs anywhere.
    Cpu,
    /// A hidden raylib window: the game's own renderer.
    Gpu,
}

#[derive(Parser)]
#[command(name = "mapshot", about = "Render bongbong maps to PNG thumbnails (docs/mapshot-prd.md)")]
struct Args {
    /// Map TOML files, or directories searched for `*.toml` (recursively,
    /// in sorted order).
    #[arg(required = true, value_name = "MAP")]
    maps: Vec<PathBuf>,

    /// Where to write the one PNG (a single map only).
    #[arg(short = 'o', long = "out", conflicts_with = "out_dir", value_name = "FILE")]
    out: Option<PathBuf>,

    /// Directory for the PNGs: a map given as a file lands at
    /// `<DIR>/<stem>.png`, a map found under a directory argument keeps
    /// its relative path with `.toml` swapped for `.png`. Without this,
    /// each PNG is written beside its map.
    #[arg(long = "out-dir", value_name = "DIR")]
    out_dir: Option<PathBuf>,

    /// Which renderer paints the field.
    #[arg(long, value_enum, default_value_t = Renderer::Cpu)]
    renderer: Renderer,

    /// Whole-pixel upscale factor (nearest neighbour), 1 = the field's own
    /// pixels.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=16))]
    scale: u32,

    /// The round seed (decimal or 0x-hex). Only cosmetics depend on it -
    /// ground and grass tiles, the frog's colour, a rolled chassis - and the
    /// fixed default keeps a batch stable from run to run.
    #[arg(long, value_parser = bongbong::parse_seed, default_value = "0xB0B5")]
    seed: u64,

    /// Stage one or two player start tanks.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=2))]
    players: u8,

    /// Player 1's chassis (else the map's `tank`, else a roll).
    #[arg(long, value_enum)]
    tank: Option<TankKind>,

    /// Player 2's chassis (else the map's `tank2`, else a roll).
    #[arg(long, value_enum)]
    tank2: Option<TankKind>,

    /// Leave the player tank(s) out: the field as authored.
    #[arg(long = "no-tanks")]
    no_tanks: bool,

    /// Flat white ground instead of the tileset.
    #[arg(long)]
    plain: bool,

    /// No drop shadows (the game draws them).
    #[arg(long = "no-shadows")]
    no_shadows: bool,

    /// Silence raylib's own log lines (warnings are shown by default).
    #[arg(long)]
    quiet: bool,

    /// Render every map on both renderers and report how far apart they
    /// are (opens a hidden window). The PNG written is still the chosen
    /// renderer's; a map beyond tolerance counts as a failure.
    #[arg(long)]
    check: bool,
}

/// One map to render and where its PNG goes.
struct Job {
    map: PathBuf,
    out: PathBuf,
}

/// Every `*.toml` under `dir`, recursively, sorted for a stable batch.
fn toml_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| format!("{}: {e}", d.display()))?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "toml") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Expand the arguments into jobs, applying the naming rules of `--out`
/// and `--out-dir`.
fn plan(args: &Args) -> Result<Vec<Job>, String> {
    let mut jobs = Vec::new();
    for given in &args.maps {
        if given.is_dir() {
            for map in toml_files(given)? {
                let out = match &args.out_dir {
                    Some(dir) => dir.join(map.strip_prefix(given).unwrap_or(&map)).with_extension("png"),
                    None => map.with_extension("png"),
                };
                jobs.push(Job { map, out });
            }
        } else {
            let out = match &args.out_dir {
                Some(dir) => dir.join(given.file_name().ok_or_else(|| format!("{}: not a file", given.display()))?).with_extension("png"),
                None => given.with_extension("png"),
            };
            jobs.push(Job { map: given.clone(), out });
        }
    }
    if let Some(out) = &args.out {
        if jobs.len() != 1 {
            return Err(format!("--out names one file but {} maps were given; use --out-dir for a batch", jobs.len()));
        }
        jobs[0].out = out.clone();
    }
    Ok(jobs)
}

/// The GPU renderer's state, created on the first job that needs it so a
/// CPU batch never opens a window.
struct Gpu {
    rl: RaylibHandle,
    thread: RaylibThread,
    sheets: GpuSheets,
}

impl Gpu {
    fn open(level: TraceLogLevel) -> Result<Self, String> {
        // The window is hidden; its size only has to be non-zero, the
        // render texture is sized per map.
        let (mut rl, thread) = sola_raylib::init().size(64, 64).title("mapshot").hidden().log_level(level).build();
        let sheets = GpuSheets::load(&mut rl, &thread)?;
        Ok(Gpu { rl, thread, sheets })
    }
}

/// raylib's own log threshold: its INFO chatter is noise in a CLI.
fn log_level(args: &Args) -> TraceLogLevel {
    if args.quiet { TraceLogLevel::LOG_NONE } else { TraceLogLevel::LOG_WARNING }
}

fn run(args: &Args) -> Result<usize, String> {
    // Before any raylib call, so decoding the sheets on the CPU path is
    // quiet too; the window builder sets it again for the GPU path.
    unsafe {
        sola_raylib::ffi::SetTraceLogLevel(log_level(args) as i32);
    }
    if !Path::new("static").is_dir() {
        return Err("no `static/` directory here: run mapshot from the repository root (or an extracted release), where the sprite sheets are".into());
    }
    let jobs = plan(args)?;
    if jobs.is_empty() {
        return Err("no maps found".into());
    }
    let options = ThumbnailOptions {
        seed: args.seed,
        players: PlayerCount::from_count(args.players as usize).expect("clap keeps players in 1..=2"),
        tank_row: args.tank.map(TankKind::row),
        tank2_row: args.tank2.map(TankKind::row),
        hide_players: args.no_tanks,
        plain_canvas: args.plain,
        shadows: !args.no_shadows,
    };

    let cpu_sheets = if args.check || args.renderer == Renderer::Cpu { Some(load_cpu_sheets()?) } else { None };
    let mut gpu = if args.check || args.renderer == Renderer::Gpu { Some(Gpu::open(log_level(args))?) } else { None };

    let mut failures = 0usize;
    for job in &jobs {
        match render_one(job, &options, args.renderer, cpu_sheets.as_ref(), gpu.as_mut(), args.scale) {
            Ok(done) => {
                print!("{} -> {} {}x{}", job.map.display(), job.out.display(), done.width, done.height);
                if let Some(c) = done.check {
                    print!("  gpu vs cpu: mean channel diff {:.3}, {:.2}% pixels off", c.mean_channel_diff, c.off_fraction * 100.0);
                    if !c.within_tolerance() {
                        failures += 1;
                        print!("  BEYOND TOLERANCE");
                    }
                }
                println!();
            }
            Err(e) => {
                failures += 1;
                eprintln!("mapshot: {}: {e}", job.map.display());
            }
        }
    }
    Ok(failures)
}

/// What one job produced.
struct Done {
    width: u32,
    height: u32,
    check: Option<Comparison>,
}

/// Render one map to its PNG.
fn render_one(
    job: &Job,
    options: &ThumbnailOptions,
    renderer: Renderer,
    cpu_sheets: Option<&BTreeMap<Sheet, Pixels>>,
    gpu: Option<&mut Gpu>,
    scale: u32,
) -> Result<Done, String> {
    let map = MapFile::load(&job.map)?;
    let game = thumbnail::stage_round(map, options);
    let (w, h) = thumbnail::field_pixels(&game);
    let cpu = cpu_sheets.map(|sheets| thumbnail::render_cpu(&game, sheets));
    let gpu_image = match gpu {
        Some(gpu) => Some(render_gpu(&mut gpu.rl, &gpu.thread, &gpu.sheets, &game)?),
        None => None,
    };
    let check = match (&cpu, &gpu_image) {
        (Some(cpu), Some(image)) => {
            let gpu: Vec<Color> = image.get_image_data().iter().map(|&c| c.into()).collect();
            Some(thumbnail::compare_pixels(&gpu, cpu.pixels()))
        }
        _ => None,
    };
    let bytes = match renderer {
        Renderer::Cpu => cpu.as_ref().expect("the CPU sheets are loaded for the CPU renderer").png_bytes(scale)?,
        Renderer::Gpu => image_png_bytes(gpu_image.expect("the window is open for the GPU renderer"), scale)?,
    };
    if let Some(parent) = job.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
    }
    std::fs::write(&job.out, bytes).map_err(|e| format!("{}: {e}", job.out.display()))?;
    Ok(Done { width: w as u32 * scale, height: h as u32 * scale, check })
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(failures) => {
            eprintln!("mapshot: {failures} map(s) failed");
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("mapshot: {e}");
            ExitCode::from(2)
        }
    }
}
