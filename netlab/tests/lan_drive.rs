//! A whole run end to end: the in-process room server behind the proxy on
//! a clean link, two headless clients, the twin, the report.
//!
//! **This file must stay the only test in its binary.** `run::run` writes
//! the process-wide tuning table (it stages `online_client_hull` and the
//! scripted magazine, and the twin puts a room's patch on it), which every
//! other test in the same process would read mid-run - cargo runs a
//! binary's tests on parallel threads. A second end-to-end run belongs in
//! a file of its own.
//!
//! The bounds are wide on purpose: the clients draw on real frame loops,
//! so a loaded machine (a parallel build, CI) drops and delays frames.
//! What is pinned is what holds on any machine - the round was played and
//! measured, and the owned hull answers within frames of the twin's.

use bongbong::level::Mission;
use bongbong::tank::TankKind;
use netlab::run::{self, RunConfig};
use netlab::script::Scenario;

#[test]
fn a_three_second_lan_drive_is_measured_and_the_own_hull_answers_like_the_twins() {
    let cfg = RunConfig {
        label: "test".into(),
        profile: "lan".into(),
        link: run::profile("lan").expect("the lan profile"),
        fps: 60.0,
        seconds: 3.0,
        scenario: Scenario::Drive,
        client_hull: true,
        map: concat!(env!("CARGO_MANIFEST_DIR"), "/maps/arena.toml").into(),
        enemies: 0,
        seed: 0xB0B5,
        tank: TankKind::Scout,
        mission: Mission::Destroy,
        remote: None,
        frames_out: None,
    };
    let report = run::run(&cfg).expect("the run");
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let online = report.online.as_ref().expect("online frames");

    // Three seconds at sixty is 180; a starved frame loop draws fewer.
    assert!(online.frames > 60, "{} frames", online.frames);
    assert_eq!(report.twin.frames, 180, "the twin draws every frame");

    // The rectangle starts once and turns twice in three seconds: the
    // changes are answered, and the owned hull answers within a couple of
    // frames of the twin - frames, not milliseconds, so a late frame is
    // not counted as latency.
    assert!(online.own_input_frames.n >= 1, "{:?}", online.own_input_frames);
    let twin = report.twin.own_input_frames.p95.expect("the twin answers");
    assert!(twin <= 2.0, "twin own input p95 {twin} frames");
    let own = online.own_input_frames.p95.expect("a latency");
    assert!(own <= twin + 2.0, "own input p95 {own} frames against the twin's {twin}");

    // The guest draws the host moving, some time after the host did.
    assert!(online.remote_lag_ms.n > 10, "{:?}", online.remote_lag_ms);
    assert!(online.remote_lag_ms.p50.is_some_and(|l| l > 0.0 && l < 1500.0), "{:?}", online.remote_lag_ms);
    assert!(online.remote_pacing.is_some_and(|p| p.frames > 10));
    assert_eq!(report.twin.remote_lag_ms.p50, Some(0.0), "the twin draws the host where the host is");

    // The tap read both directions of both connections.
    let tap = report.tap.as_ref().expect("the tap in-process");
    assert_eq!(tap.tap_failed, None);
    assert!(tap.intents > 60 && tap.snapshots > 60, "{} intents, {} snapshots", tap.intents, tap.snapshots);
    assert!(tap.server_hold_ms.n > 20);
    assert!(tap.snapshot_gap_ms.p50.is_some_and(|g| g > 5.0 && g < 250.0), "{:?}", tap.snapshot_gap_ms);
    assert!(tap.up_bytes_per_s > 0.0 && tap.down_bytes_per_s > tap.up_bytes_per_s);

    // Both seats report their own counters, frame by frame too; owning
    // the hull, nothing was corrected.
    assert_eq!(report.seats.len(), 2);
    assert!(report.seats.iter().all(|s| s.seat.is_some() && s.rtt_p50_ms.is_some()));
    assert_eq!(report.seats.iter().map(|s| s.nudges + s.snaps).sum::<u32>(), 0);
    assert_eq!(report.series.len(), 2);
    for (series, seat) in report.series.iter().zip(&report.seats) {
        assert_eq!(series.frames.len(), seat.frames, "a row per recorded frame");
        assert!(series.frames.iter().any(|f| f.link.rtt_ms.is_some()), "the round trip is in the series");
    }
    assert_eq!(report.max_shells, Some(run::SCRIPT_MAX_SHELLS as i32), "in-process, the magazine is pinned");
    assert!(["local", "close", "far"].contains(&report.verdict.as_str()));
}
