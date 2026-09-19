//! `bongbong-server`: the online co-op room server (docs/online-coop-prd.md
//! §4.7, §4.13). `--listen 127.0.0.1:4848 --pod A --insecure` is the local
//! run (`just run-server`); the container runs the same binary on
//! `0.0.0.0`. SIGTERM drains: no new rooms, `/health` 503, the rounds in
//! progress finish, exit when the last ends or after `DRAIN_MAX`; Ctrl-C
//! exits at once.

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::process::ExitCode;

use bongbong_server::hub::DRAIN_MAX;
use bongbong_server::{Config, Server};
use clap::Parser;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "bongbong-server", about = "The bongbong online co-op room server")]
struct Args {
    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:4848")]
    listen: SocketAddr,
    /// This pod's letter, the first letter of every room code it mints.
    /// Defaults to A on a loopback listen.
    #[arg(long)]
    pod: Option<char>,
    /// Plain ws:// is expected (a local run). Refused on a non-loopback
    /// address unless --pod is explicit too.
    #[arg(long)]
    insecure: bool,
    /// The most rooms this pod holds at once.
    #[arg(long, default_value_t = 200)]
    max_rooms: usize,
}

fn config(args: Args) -> Result<Config, String> {
    let loopback = args.listen.ip().is_loopback();
    if args.insecure && !loopback && args.pod.is_none() {
        return Err(format!("--insecure on {} needs an explicit --pod", args.listen));
    }
    let pod = match args.pod {
        Some(c) if bongbong_server::code::pod_letter_valid(c) => c,
        Some(c) => return Err(format!("--pod {c:?} is not a capital letter or a digit")),
        None if loopback => 'A',
        None => return Err(format!("--pod is required on {}", args.listen)),
    };
    if args.max_rooms == 0 {
        return Err("--max-rooms must be at least 1".into());
    }
    Ok(Config { listen: args.listen, pod, insecure: args.insecure, max_rooms: args.max_rooms })
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let config = match config(Args::parse()) {
        Ok(c) => c,
        Err(e) => {
            error!("{e}");
            return ExitCode::from(2);
        }
    };
    let server = match Server::bind(config.clone()).await {
        Ok(s) => s,
        Err(e) => {
            error!("cannot listen on {}: {e}", config.listen);
            return ExitCode::from(1);
        }
    };
    info!(
        addr = %server.addr,
        pod = %config.pod,
        insecure = config.insecure,
        max_rooms = config.max_rooms,
        protocol = bongbong::net::PROTOCOL_VERSION,
        "listening; ws://{}/ws, /health, /metrics",
        server.addr
    );
    let hub = server.hub.clone();
    let shutdown = async move {
        tokio::select! {
            _ = terminate() => {
                hub.begin_drain();
                info!(rooms = hub.room_count(), "SIGTERM: draining, no new rooms; /health reports 503");
                tokio::select! {
                    _ = hub.drained() => info!("drained: every room ended"),
                    _ = tokio::time::sleep(DRAIN_MAX) => info!(rooms = hub.room_count(), "drain limit reached, exiting"),
                    _ = tokio::signal::ctrl_c() => info!("Ctrl-C during the drain, exiting"),
                }
            }
            _ = tokio::signal::ctrl_c() => info!("Ctrl-C, exiting"),
        }
        hub.shutdown();
    };
    match server.run(shutdown).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("server error: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(unix)]
async fn terminate() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut sigterm) => {
            sigterm.recv().await;
        }
        Err(e) => {
            error!("cannot listen for SIGTERM: {e}");
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(not(unix))]
async fn terminate() {
    std::future::pending::<()>().await;
}
