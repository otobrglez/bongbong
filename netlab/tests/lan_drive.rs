//! A whole run end to end: the in-process room server behind the proxy on
//! a clean link, two headless clients, the twin, the report.

use bongbong::level::Mission;
use bongbong::tank::TankKind;
use netlab::run::{self, RunConfig};
use netlab::script::Scenario;

#[test]
fn a_three_second_lan_drive_is_measured_and_the_own_hull_answers_within_two_frames() {
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
    };
    let report = run::run(&cfg).expect("the run");
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let online = report.online.as_ref().expect("online frames");

    // Three seconds at sixty, give or take the frames a busy machine drops.
    assert!(online.frames > 120, "{} frames", online.frames);
    assert_eq!(report.twin.frames, 180, "the twin draws every frame");

    // The rectangle starts once and turns twice in three seconds: every
    // change is answered, and the owned hull answers as a local one does.
    assert!(online.own_input_frames.n >= 2, "{:?}", online.own_input_frames);
    let own = online.own_input_frames.p95.expect("a latency");
    assert!(own <= 2.0, "own input p95 {own} frames");
    assert!(report.twin.own_input_frames.p95.is_some_and(|t| t <= 2.0));

    // The guest draws the host moving, some time after the host did.
    assert!(online.remote_lag_ms.n > 30, "{:?}", online.remote_lag_ms);
    assert!(online.remote_lag_ms.p50.is_some_and(|l| l > 0.0 && l < 500.0));
    assert!(online.remote_pacing.is_some_and(|p| p.frames > 30));
    assert_eq!(report.twin.remote_lag_ms.p50, Some(0.0), "the twin draws the host where the host is");

    // The tap read both directions of both connections.
    let tap = report.tap.as_ref().expect("the tap in-process");
    assert_eq!(tap.tap_failed, None);
    assert!(tap.intents > 100 && tap.snapshots > 100, "{} intents, {} snapshots", tap.intents, tap.snapshots);
    assert!(tap.server_hold_ms.n > 50);
    assert!(tap.snapshot_gap_ms.p50.is_some_and(|g| g > 5.0 && g < 60.0));
    assert!(tap.up_bytes_per_s > 0.0 && tap.down_bytes_per_s > tap.up_bytes_per_s);

    // Both seats report their own counters; owning the hull, nothing was
    // corrected.
    assert_eq!(report.seats.len(), 2);
    assert!(report.seats.iter().all(|s| s.seat.is_some() && s.rtt_p50_ms.is_some()));
    assert_eq!(report.seats.iter().map(|s| s.nudges + s.snaps).sum::<u32>(), 0);
    assert!(["local", "close", "far"].contains(&report.verdict.as_str()));
}
