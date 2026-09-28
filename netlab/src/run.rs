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
use bongbong::tuning;

use crate::client::{Rendezvous, Role, SeatPlan, SeatRun, run_seat};
use crate::link::Impairment;
use crate::metrics::{self, Metrics, Stat, View};
use crate::proxy::{self, Clock};
use crate::report::{self, Report, SeatSummary};
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
/// without its `/ws`), as `--rooms` takes it.
pub fn remote_url(remote: &str) -> String {
    let base = remote.trim().trim_end_matches('/');
    let base = base.strip_suffix("/ws").unwrap_or(base);
    socket_url(&RoomsHost::overriding(base))
}

/// A magazine no scripted shooter empties: every press the script makes
/// is one the room can answer, so a press and its `Fired` pair up in
/// order and a refusal never shifts the ledger. Set for the room, the
/// clients and the twin alike.
pub const SCRIPT_MAX_SHELLS: f64 = 100.0;

/// Put the run's knobs on the process's table before any round is built:
/// `online_client_hull`, which the clients read when they open, and the
/// scripted shooters' magazine. The in-process room server keeps the
/// table it first sees as its base, and the twin is built on it too.
fn stage_tuning(client_hull: bool) -> Result<(), String> {
    let mut t = tuning::current();
    t.set("online_client_hull", if client_hull { 1.0 } else { 0.0 })?;
    t.set("max_shells", SCRIPT_MAX_SHELLS)?;
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
        interp_delay_ms: Stat::of(&run.samples.iter().filter_map(|f| f.interp_delay_ms).collect::<Vec<_>>()),
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
        s.interp_jitter_ms = end.jitter_ms;
        s.interp_interval_ms = end.interval_ms;
        let start = run.interp_start.map_or(0, |i| i.extrapolated_frames);
        s.extrapolated_frames = end.extrapolated_frames.saturating_sub(start);
    }
    if let Some(r) = run.rtt {
        s.rtt_p50_ms = Some(r.rtt_ms);
        s.rtt_p95_ms = Some(r.rtt_p95_ms);
        s.rtt_min_ms = Some(r.rtt_min_ms);
    }
    s
}

/// Play the run and measure it.
pub fn run(cfg: &RunConfig) -> Result<Report, String> {
    stage_tuning(cfg.client_hull)?;
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
                let config = bongbong_server::Config { listen: "127.0.0.1:0".parse().expect("an address"), insecure: true, max_rooms: 4 };
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

    let mut errors: Vec<String> = runs.iter().filter_map(|r| r.error.clone()).collect();
    let window = {
        let from = runs.iter().filter_map(|r| r.t0_ms).fold(f64::INFINITY, f64::min);
        let to = runs.iter().filter_map(|r| r.samples.last().map(|s| s.t_ms)).fold(f64::NEG_INFINITY, f64::max);
        (from.is_finite() && to.is_finite()).then_some((from, to))
    };
    let tap = match (tap, window) {
        (Some(log), Some((from, to))) => {
            let log = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
            Some(report::tap_metrics(&log, from, to))
        }
        _ => None,
    };
    runtime.shutdown_timeout(Duration::from_millis(200));

    let twin_frames = twin::run(&TwinPlan {
        map,
        mission: cfg.mission,
        seed: cfg.seed,
        scenario: cfg.scenario,
        fps: cfg.fps,
        seconds: cfg.seconds,
    });
    let twin = metrics::measure(&View {
        host: &twin_frames,
        guest: &twin_frames,
        host_t0_ms: 0.0,
        host_seat: 0,
        guest_seat: 1,
        scenario: cfg.scenario,
    });
    let online: Option<Metrics> = match (host.t0_ms, host.samples.is_empty() || guest.samples.is_empty()) {
        (Some(t0), false) => Some(metrics::measure(&View {
            host: &host.samples,
            guest: &guest.samples,
            host_t0_ms: t0,
            host_seat: host.seat.unwrap_or(0) as usize,
            guest_seat: guest.seat.unwrap_or(1) as usize,
            scenario: cfg.scenario,
        })),
        _ => {
            errors.push("no online frames were recorded".into());
            None
        }
    };
    let seats: Vec<SeatSummary> = runs.iter().map(seat_summary).collect();
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
        online,
        twin,
        seats,
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
}
