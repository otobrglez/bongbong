//! `bongbong-server`: the online co-op room server (docs/online-coop-prd.md
//! §4.7, §4.13). `--listen 127.0.0.1:4848 --insecure` is the local run
//! (`just run-server`); the container runs the same binary on `0.0.0.0`.
//! **One instance holds every room**, so there is nothing to configure
//! about where a room lives. SIGTERM drains: no new rooms, `/health` 503,
//! the rounds in progress finish, exit when the last ends or after
//! `DRAIN_MAX`; Ctrl-C exits at once.

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
    /// Plain ws:// with no TLS terminator in front is expected here (a
    /// local run). Deployed, the ingress holds the certificate and this
    /// stays off; either way it is only recorded in the start-up line.
    #[arg(long)]
    insecure: bool,
    /// The most rooms this server holds at once.
    #[arg(long, default_value_t = 200)]
    max_rooms: usize,
    /// The dev tools' loopback port (`bbmcp rooms`), 0 to turn them off.
    /// Only a `--features dev-tools` build has them at all, and the
    /// listener never leaves 127.0.0.1.
    #[cfg(feature = "dev-tools")]
    #[arg(long, default_value_t = bongbong::devserver::ROOMS_DEV_PORT)]
    dev_port: u16,
}

fn config(args: Args) -> Result<Config, String> {
    if args.max_rooms == 0 {
        return Err("--max-rooms must be at least 1".into());
    }
    Ok(Config { listen: args.listen, insecure: args.insecure, max_rooms: args.max_rooms })
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_target(false)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let args = Args::parse();
    #[cfg(feature = "dev-tools")]
    let dev_port = args.dev_port;
    let config = match config(args) {
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
        insecure = config.insecure,
        max_rooms = config.max_rooms,
        protocol = bongbong::net::PROTOCOL_VERSION,
        "listening; ws://{}/ws, /health, /metrics",
        server.addr
    );
    // The dev tools, on loopback and only in a build that has them.
    #[cfg(feature = "dev-tools")]
    if dev_port != 0 {
        let hub = server.hub.clone();
        let addr = SocketAddr::from(([127, 0, 0, 1], dev_port));
        tokio::spawn(async move { bongbong_server::devserver::serve(hub, addr).await });
    }
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
