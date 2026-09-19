//! The HTTP surface (docs/online-coop-prd.md §4.13): `GET /health` (200
//! while taking rooms, 503 while draining), `GET /metrics` (Prometheus
//! text) and `GET /ws`, the WebSocket every message travels over. TLS is
//! the ingress's; the server speaks plain `ws`.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::net::TcpListener;

use crate::conn;
use crate::hub::Hub;
use crate::metrics::Metrics;

/// What a running server was started with.
#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    /// The pod letter, the first of every code this server mints.
    pub pod: char,
    /// Plain `ws://` is expected here (a local run); logged, and the CLI
    /// refuses it on a non-loopback address without an explicit pod.
    pub insecure: bool,
    /// The most rooms this pod holds at once.
    pub max_rooms: usize,
}

/// A bound listener and the hub behind it; `run` serves until the
/// shutdown future resolves.
pub struct Server {
    pub addr: SocketAddr,
    pub hub: Arc<Hub>,
    pub config: Config,
    listener: TcpListener,
}

impl Server {
    /// Bind `config.listen` (port 0 picks an ephemeral one: `addr` has
    /// the real one).
    pub async fn bind(config: Config) -> io::Result<Server> {
        let listener = TcpListener::bind(config.listen).await?;
        let addr = listener.local_addr()?;
        let hub = Hub::new(config.pod, config.max_rooms, Arc::new(Metrics::new()));
        Ok(Server { addr, hub, config, listener })
    }

    /// Serve until `shutdown` resolves, then finish the open connections.
    pub async fn run(self, shutdown: impl Future<Output = ()> + Send + 'static) -> io::Result<()> {
        axum::serve(self.listener, router(self.hub)).with_graceful_shutdown(shutdown).await
    }
}

fn router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/ws", get(ws))
        .with_state(hub)
}

async fn health(State(hub): State<Arc<Hub>>) -> Response {
    if hub.draining() {
        (StatusCode::SERVICE_UNAVAILABLE, "draining\n").into_response()
    } else {
        (StatusCode::OK, "ok\n").into_response()
    }
}

async fn metrics(State(hub): State<Arc<Hub>>) -> Response {
    let text = hub.metrics.render(hub.counts(), hub.draining(), hub.pod);
    ([("content-type", "text/plain; version=0.0.4; charset=utf-8")], text).into_response()
}

async fn ws(ws: WebSocketUpgrade, State(hub): State<Arc<Hub>>) -> Response {
    ws.on_upgrade(move |socket| conn::run(socket, hub))
}
