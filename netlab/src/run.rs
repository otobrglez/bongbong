//! One run: the server and the proxy (or a remote server), two scripted
//! seats, the local twin, the metrics and the verdict.

use std::path::PathBuf;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use bongbong::level::{Mission, SpawnKind};
use bongbong::map::MapFile;
use bongbong::net::client::RoomSetup;
use bongbong::net::rooms::{RoomsHost, socket_url};
use bongbong::tank::TankKind;
use bongbong::tuning::{self, Tuning};

use crate::client::{Rendezvous, Role, SeatPlan, SeatRun, run_seat};
use crate::link::Impairment;
use crate::metrics::{self, Metrics, Stat, View, WireFired};
use crate::proxy::{self, Clock};
use crate::report::{self, Report, SeatSeries, SeatSummary};
use crate::sample::FrameSample;
use crate::script::Scenario;
use crate::twin::{self, TwinPlan};

/// The shipped link profiles: one-way delay, jitter, loss.
pub const PROFILES: [(&str, f64, f64, f64); 5] = [
    ("lan", 0.0, 0.0, 0.0),
    ("good", 15.0, 3.0, 0.0),
    ("typical", 40.0, 10.0, 0.0),
    ("mobile", 60.0, 25.0, 0.01),
    ("bad", 100.0, 40.0, 0.03),
];

/// A profile's link by name, with the default retransmit timeout.
pub fn profile(name: &str) -> Option<Impairment> {
    PROFILES.iter().find(|p| p.0 == name).map(|&(_, delay, jitter, loss)| Impairment {
        delay_ms: delay,
        jitter_ms: jitter,
        loss,
        rto_ms: Impairment::default_rto_ms(delay),
        nagle: false,
        blackhole_after_ms: None,
    })
}

/// Everything a run is asked to do.
#[derive(Clone, Debug)]
pub struct RunConfig {
    pub label: String,
    pub profile: String,
    pub link: Impairment,
    pub fps: f64,
    pub seconds: f64,
    pub scenario: Scenario,
    pub client_hull: bool,
    pub map: PathBuf,
    pub enemies: usize,
    pub seed: u64,
    pub tank: TankKind,
    pub mission: Mission,
    /// A room server to dial instead of the in-process one (no proxy, no
    /// tap).
    pub remote: Option<String>,
    /// Where to write every recorded frame - each seat's and the twin's - as
    /// JSON.
    pub frames_out: Option<PathBuf>,
}

/// The map the run plays: the file, with the band plan, the enemy count
/// and both seats' chassis pinned, so the room and the twin build the
/// same round.
pub fn prepare_map(cfg: &RunConfig) -> Result<MapFile, String> {
    let mut map = MapFile::load(&cfg.map)?;
    map.tanks = Some(cfg.enemies as u32);
    map.spawn.kind = SpawnKind::Band;
    map.tank = Some(cfg.tank);
    map.tank2 = Some(cfg.tank);
    Ok(map)
}

/// The socket URL a remote server is dialled at: a rooms host (with or
/// without its `/ws`), as `--rooms` takes it. A WebSocket only speaks
/// `ws://` and `wss://`, so a page's `http(s)://` is read as its socket's
/// scheme, and a bare `host:port` gets `ws://` on this machine (what
/// `just run-server` listens on) and `wss://` anywhere else.
pub fn remote_url(remote: &str) -> String {
    let base = remote.trim().trim_end_matches('/');
    let base = base.strip_suffix("/ws").unwrap_or(base);
    let base = match base.split_once("://") {
        Some(("http", rest)) => format!("ws://{rest}"),
        Some(("https", rest)) => format!("wss://{rest}"),
        Some(_) => base.to_string(),
        None => {
            let host = base.split(['/', ':']).next().unwrap_or("");
            let local = host == "localhost" || host.starts_with("127.") || base.starts_with("[::1]");
            format!("{}://{base}", if local { "ws" } else { "wss" })
        }
    };
    socket_url(&RoomsHost::overriding(&base))
}

/// A magazine no scripted shooter empties: every press the script makes
/// is one the room can answer, so no press is refused for ammo. Set for
/// the in-process room, the clients and the twin alike.
pub const SCRIPT_MAX_SHELLS: f64 = 100.0;

/// How a run keeps every scripted press answerable: an in-process room
/// shares this process's table, so its magazine is pinned at
/// `SCRIPT_MAX_SHELLS` for the room, the clients and the twin; a remote
/// room keeps its own, so nothing is pinned anywhere and each script stops
/// tapping after the shells a tank starts with (`max_shells` as this
/// build ships it - the remote room's own is not readable from here).
pub fn magazine(remote: bool) -> (Option<i32>, Option<u32>) {
    if remote {
        (None, Some(Tuning::DEFAULT.max_shells.max(0) as u32))
    } else {
        (Some(SCRIPT_MAX_SHELLS as i32), None)
    }
}

/// Put the run's knobs on the process's table before any round is built:
/// `online_client_hull`, which the clients read when they open, and the
/// pinned magazine, if any. The in-process room server keeps the table it
/// first sees as its base, and the twin is built on it too.
fn stage_tuning(client_hull: bool, max_shells: Option<i32>) -> Result<(), String> {
    let mut t = tuning::current();
    t.set("online_client_hull", if client_hull { 1.0 } else { 0.0 })?;
    if let Some(n) = max_shells {
        t.set("max_shells", n as f64)?;
    }
    tuning::replace_now(t);
    Ok(())
}

fn seat_summary(run: &SeatRun) -> SeatSummary {
    let mut s = SeatSummary {
        seat: run.seat,
        frames: run.samples.len(),
        ended_early: run.ended_early,
        outcome: run.outcome.clone(),
        error: run.error.clone(),
        interp_delay_ms: Stat::of(&run.samples.iter().filter_map(|f| f.link.map(|l| l.delay_ms)).collect::<Vec<_>>()),
        playout_rate: Stat::of(&run.samples.iter().filter_map(|f| f.link.map(|l| l.rate)).collect::<Vec<_>>()),
        dial_retries: run.dial_retries,
        ..SeatSummary::default()
    };
    if let Some(p) = run.prediction {
        s.nudges = p.nudges;
        s.snaps = p.snaps;
        s.max_error_px = p.max_error_px;
        s.lead_up = p.lead_up;
        s.lead_down = p.lead_down;
        s.shots_drawn = p.shots_drawn;
        s.shots_refused = p.shots_refused;
        s.crossings = p.crossings;
        s.crossings_hit = p.crossings_hit;
        s.crossings_missed = p.crossings_missed;
    }
    if let Some(end) = run.interp_end {
        let start = run.interp_start.unwrap_or_default();
        s.interp_jitter_ms = end.jitter_ms;
        s.interp_interval_ms = end.interval_ms;
        s.extrapolated_frames = end.extrapolated_frames.saturating_sub(start.extrapolated_frames);
        s.lateness_p50_ms = end.lateness_p50_ms;
        s.lateness_p95_ms = end.lateness_p95_ms;
        s.stalls = end.stalls.saturating_sub(start.stalls);
        s.interp_corrections = end.corrections.saturating_sub(start.corrections);
        s.interp_correction_p95_px = end.error_p95_px;
    }
    if let Some(r) = run.rtt {
        s.rtt_p50_ms = Some(r.rtt_ms);
        s.rtt_p95_ms = Some(r.rtt_p95_ms);
        s.rtt_min_ms = Some(r.rtt_min_ms);
    }
    s
}

/// Every frame a run recorded - each seat's and the twin's - with what
/// the metrics read beside the frames: what `--frames-out` writes, and what
/// `netlab replay` measures again (`measure_dump`).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FrameDump {
    pub scenario: Scenario,
    pub host_seat: usize,
    pub guest_seat: usize,
    /// The host's script start on the process clock.
    pub host_t0_ms: Option<f64>,
    /// The tap's reading of each host press (in-process only).
    pub wire: Option<WireFired>,
    pub host: Vec<FrameSample>,
    pub guest: Vec<FrameSample>,
    pub twin: Vec<FrameSample>,
}

/// The metrics of a run's frames: the online view (`None` when either
/// seat recorded nothing) and the twin's.
pub fn measure_dump(d: &FrameDump) -> (Option<Metrics>, Metrics) {
    let twin = metrics::measure(&View {
        host: &d.twin,
        guest: &d.twin,
        host_t0_ms: 0.0,
        host_seat: 0,
        guest_seat: 1,
        scenario: d.scenario,
        wire: None,
    });
    let online = match (d.host_t0_ms, d.host.is_empty() || d.guest.is_empty()) {
        (Some(t0), false) => Some(metrics::measure(&View {
            host: &d.host,
            guest: &d.guest,
            host_t0_ms: t0,
            host_seat: d.host_seat,
            guest_seat: d.guest_seat,
            scenario: d.scenario,
            wire: d.wire.as_ref(),
        })),
        _ => None,
    };
    (online, twin)
}

/// Play the run and measure it.
///
/// It stages knobs on this process's tuning table (`stage_tuning`) and the
/// twin puts a room's patch on it, so a caller - a test included - runs
/// one at a time, alone in its process.
pub fn run(cfg: &RunConfig) -> Result<Report, String> {
    let (max_shells, tap_limit) = magazine(cfg.remote.is_some());
    stage_tuning(cfg.client_hull, max_shells)?;
    let map = prepare_map(cfg)?;
    let map_toml = map.to_toml_string()?;
    let clock = Clock::new();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| format!("the runtime: {e}"))?;

    let (url, tap) = match &cfg.remote {
        Some(remote) => (remote_url(remote), None),
        None => {
            let link = cfg.link;
            let seed = cfg.seed;
            let (addr, log) = runtime.block_on(async move {
                let config = bongbong_server::Config { listen: "127.0.0.1:0".parse().expect("an address"), admin_listen: None, insecure: true, max_rooms: 4 };
                let server = bongbong_server::Server::bind(config).await.map_err(|e| format!("bind the room server: {e}"))?;
                let upstream = server.addr;
                tokio::spawn(server.run(std::future::pending()));
                let proxy = proxy::start(upstream, link, seed, clock).await.map_err(|e| format!("bind the proxy: {e}"))?;
                Ok::<_, String>((proxy.addr, proxy.log))
            })?;
            (format!("ws://{addr}/ws"), Some(log))
        }
    };

    let meet = Arc::new(Rendezvous::default());
    let stamp = format!("{}-{}", std::process::id(), clock.epoch.elapsed().as_nanos());
    let setup = RoomSetup { map: "netlab".into(), map_toml: Some(map_toml), mission: cfg.mission, seed: Some(cfg.seed) };
    let plans = [
        SeatPlan {
            url: url.clone(),
            role: Role::Host(setup),
            script_seat: 0,
            scenario: cfg.scenario,
            fps: cfg.fps,
            seconds: cfg.seconds,
            client_hull: cfg.client_hull,
            seats: 2,
            token: format!("netlab-host-{stamp}"),
            tap_limit,
        },
        SeatPlan {
            url,
            role: Role::Guest,
            script_seat: 1,
            scenario: cfg.scenario,
            fps: cfg.fps,
            seconds: cfg.seconds,
            client_hull: cfg.client_hull,
            seats: 2,
            token: format!("netlab-guest-{stamp}"),
            tap_limit,
        },
    ];
    let handles: Vec<_> = plans
        .into_iter()
        .map(|plan| {
            let meet = meet.clone();
            std::thread::Builder::new()
                .name(format!("netlab-seat-{}", plan.script_seat))
                .spawn(move || run_seat(plan, clock, &meet))
                .expect("a seat thread")
        })
        .collect();
    let runs: Vec<SeatRun> = handles.into_iter().map(|h| h.join().unwrap_or_default()).collect();
    let (host, guest) = (&runs[0], &runs[1]);
    let (host_seat, guest_seat) = (host.seat.unwrap_or(0), guest.seat.unwrap_or(1));

    let mut errors: Vec<String> = runs.iter().filter_map(|r| r.error.clone()).collect();
    let window = {
        let from = runs.iter().filter_map(|r| r.t0_ms).fold(f64::INFINITY, f64::min);
        let to = runs.iter().filter_map(|r| r.samples.last().map(|s| s.t_ms)).fold(f64::NEG_INFINITY, f64::max);
        (from.is_finite() && to.is_finite()).then_some((from, to))
    };
    let (tap, wire) = match (tap, window) {
        (Some(log), Some((from, to))) => {
            let log = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
            let presses = metrics::presses(&host.samples, host_seat as usize);
            (Some(report::tap_metrics(&log, from, to)), report::wire_fired(&log, &presses, host_seat, guest_seat))
        }
        _ => (None, None),
    };
    runtime.shutdown_timeout(Duration::from_millis(200));

    let dump = FrameDump {
        scenario: cfg.scenario,
        host_seat: host_seat as usize,
        guest_seat: guest_seat as usize,
        host_t0_ms: host.t0_ms,
        wire,
        host: host.samples.clone(),
        guest: guest.samples.clone(),
        twin: twin::run(&TwinPlan {
            map,
            mission: cfg.mission,
            seed: cfg.seed,
            scenario: cfg.scenario,
            fps: cfg.fps,
            seconds: cfg.seconds,
            tap_limit,
        }),
    };
    let (online, twin) = measure_dump(&dump);
    if online.is_none() {
        errors.push("no online frames were recorded".into());
    }
    if let Some(path) = &cfg.frames_out {
        let text = serde_json::to_string(&dump).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let seats: Vec<SeatSummary> = runs.iter().map(seat_summary).collect();
    let series: Vec<SeatSeries> = runs.iter().map(|r| SeatSeries::of(r.seat, &r.samples)).collect();
    let (verdict, misses) = match &online {
        Some(online) => report::verdict(online, &twin, &seats, cfg.client_hull),
        None => ("failed".to_string(), Vec::new()),
    };
    Ok(Report {
        label: cfg.label.clone(),
        profile: cfg.profile.clone(),
        link: cfg.link,
        scenario: cfg.scenario,
        client_hull: cfg.client_hull,
        fps: cfg.fps,
        seconds: cfg.seconds,
        seed: cfg.seed,
        enemies: cfg.enemies,
        tank: cfg.tank.name().to_string(),
        map: cfg.map.display().to_string(),
        remote: cfg.remote.clone(),
        max_shells,
        tap_limit,
        online,
        twin,
        seats,
        series,
        tap,
        verdict,
        misses,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profiles_are_the_prd_numbers() {
        let typical = profile("typical").unwrap();
        assert_eq!((typical.delay_ms, typical.jitter_ms, typical.loss), (40.0, 10.0, 0.0));
        assert_eq!(typical.rto_ms, 200.0);
        let bad = profile("bad").unwrap();
        assert_eq!((bad.delay_ms, bad.jitter_ms, bad.loss, bad.rto_ms), (100.0, 40.0, 0.03, 200.0));
        assert!(profile("nope").is_none());
    }

    #[test]
    fn a_remote_host_is_dialled_at_its_ws_path() {
        assert_eq!(remote_url("wss://rooms.bongbong.io/pr-48"), "wss://rooms.bongbong.io/pr-48/ws");
        assert_eq!(remote_url("wss://rooms.bongbong.io/pr-48/ws"), "wss://rooms.bongbong.io/pr-48/ws");
        assert_eq!(remote_url("ws://127.0.0.1:4848/"), "ws://127.0.0.1:4848/ws");
    }

    /// A socket is only ever `ws://` or `wss://`: a page's scheme is read as
    /// its socket's, and a bare host gets one - plain on this machine.
    #[test]
    fn a_remote_host_without_a_socket_scheme_gets_one() {
        assert_eq!(remote_url("127.0.0.1:4848"), "ws://127.0.0.1:4848/ws");
        assert_eq!(remote_url("localhost:4848/ws"), "ws://localhost:4848/ws");
        assert_eq!(remote_url("[::1]:4848"), "ws://[::1]:4848/ws");
        assert_eq!(remote_url("http://127.0.0.1:4848"), "ws://127.0.0.1:4848/ws");
        assert_eq!(remote_url("https://rooms.bongbong.io/pr-48/"), "wss://rooms.bongbong.io/pr-48/ws");
        assert_eq!(remote_url("rooms.bongbong.io/pr-48"), "wss://rooms.bongbong.io/pr-48/ws");
    }

    /// An in-process room shares the process's table, so its magazine is
    /// pinned for everyone and no tap is capped; a remote room's is its
    /// own, so nothing is pinned and the taps stop at the shells a tank
    /// starts with.
    #[test]
    fn a_remote_room_pins_no_magazine_and_caps_the_taps() {
        assert_eq!(magazine(false), (Some(SCRIPT_MAX_SHELLS as i32), None));
        let (pinned, cap) = magazine(true);
        assert_eq!(pinned, None);
        assert_eq!(cap, Some(Tuning::DEFAULT.max_shells as u32));
    }
}
