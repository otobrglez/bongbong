//! `--remote` against a room server that is not listening yet: the seats
//! dial again until it is, rather than giving the run up on the first
//! refused connection - the room server started beside netlab (`just
//! run-server`, a container coming up) takes a moment to bind.
//!
//! **This file must stay the only test in its binary**: `run::run` writes
//! the process-wide tuning table (see `lan_drive.rs`).

use std::time::Duration;

use bongbong::level::Mission;
use bongbong::tank::TankKind;
use netlab::run::{self, RunConfig};
use netlab::script::Scenario;

#[test]
fn a_remote_server_that_starts_late_is_dialled_until_it_answers() {
    // A port nothing listens on yet.
    let port = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port").local_addr().expect("its address").port();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a runtime");
    let handle = runtime.handle().clone();
    let late = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        handle.block_on(async move {
            let listen = format!("127.0.0.1:{port}").parse().expect("an address");
            let config = bongbong_server::Config { listen, insecure: true, max_rooms: 4 };
            let server = bongbong_server::Server::bind(config).await.expect("the room server binds");
            tokio::spawn(server.run(std::future::pending()));
        });
    });

    let cfg = RunConfig {
        label: "late".into(),
        profile: "remote".into(),
        link: run::profile("lan").expect("the lan profile"),
        fps: 60.0,
        seconds: 1.5,
        scenario: Scenario::Drive,
        client_hull: true,
        map: concat!(env!("CARGO_MANIFEST_DIR"), "/maps/arena.toml").into(),
        enemies: 0,
        seed: 0xB0B5,
        tank: TankKind::Scout,
        mission: Mission::Destroy,
        // A bare host and port: dialled as ws:// on this machine.
        remote: Some(format!("127.0.0.1:{port}")),
        frames_out: None,
    };
    let report = run::run(&cfg).expect("the run");
    late.join().expect("the server thread");
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.online.is_some(), "the round was played");
    assert!(report.seats[0].dial_retries > 0, "the host dialled before the server listened: {:?}", report.seats[0]);
    assert_eq!(report.max_shells, None, "a remote room's magazine is its own");
    assert!(report.tap.is_none(), "no proxy, no tap");
    drop(runtime);
}
