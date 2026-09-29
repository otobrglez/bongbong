//! A path that dies without closing, end to end: the proxy swallows
//! everything both ways two seconds in and hangs up on nobody, which is
//! what a stalled route or a dropped mobile network does to a WebSocket.
//! Nothing ever arrives to say the room is gone, so only the client's own
//! keep-alive (`net::client::ROOM_SILENT_AFTER`) can: both clients must
//! report the room lost within that of the silence starting, and the run
//! must end rather than wait on sockets that will never speak again.
//!
//! **This file must stay the only test in its binary**, for the reason
//! `lan_drive.rs` gives: `run::run` writes the process-wide tuning table.

use std::time::{Duration, Instant};

use bongbong::level::Mission;
use bongbong::net::client::ROOM_SILENT_AFTER;
use bongbong::tank::TankKind;
use netlab::link::Impairment;
use netlab::run::{self, RunConfig};
use netlab::script::Scenario;

#[test]
fn a_link_that_goes_silent_is_noticed_by_both_clients_and_the_run_ends() {
    let silent_at = 2.0;
    let cfg = RunConfig {
        label: "silent".into(),
        profile: "lan".into(),
        link: Impairment { blackhole_after_ms: Some(silent_at * 1000.0), ..run::profile("lan").expect("the lan profile") },
        fps: 60.0,
        seconds: 20.0,
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
    let started = Instant::now();
    let report = run::run(&cfg).expect("the run");
    let took = started.elapsed();
    // Both seats end on the keep-alive's own reason.
    assert_eq!(report.errors.len(), 2, "{:?}", report.errors);
    assert!(report.errors.iter().all(|e| e.contains("stopped answering")), "{:?}", report.errors);
    // Well before the script's twenty seconds: the silence plus the
    // keep-alive, the twin's own run and a loaded machine's slack.
    let bound = Duration::from_secs_f64(silent_at) + ROOM_SILENT_AFTER + Duration::from_secs(8);
    assert!(took < bound, "the run took {took:?} against a silence at {silent_at} s");
}
