//! The HTTP surface (docs/online-coop-prd.md §4.13), on two listeners.
//! The public one serves `GET /ws` alone, the WebSocket every message
//! travels over. The admin one serves the operator's routes: `GET /health`
//! (liveness: 200 for as long as the server answers, drain or not),
//! `GET /ready` (readiness: 200 while taking rooms, 503 from the start of
//! the drain) and `GET /metrics` (Prometheus text). Deployed, only the
//! public port is routed from outside; the admin port is the kubelet's
//! and Prometheus's. TLS is the ingress's; the server speaks plain `ws`.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{State, WebSocketUpgrade};
use axum::serve::ListenerExt;
use tracing::warn;
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
    /// Where the admin routes (`/health`, `/ready`, `/metrics`) are
    /// served, apart from `listen`; `None` serves them nowhere.
    pub admin_listen: Option<SocketAddr>,
    /// Plain `ws://` with nothing terminating TLS in front is expected
    /// here (a local run). Logged, so a start-up line says which it was.
    pub insecure: bool,
    /// The most rooms this server holds at once.
    pub max_rooms: usize,
}

/// A bound listener and the hub behind it; `run` serves until the
/// shutdown future resolves.
pub struct Server {
    pub addr: SocketAddr,
    /// The bound admin listener's address, when there is one.
    pub admin_addr: Option<SocketAddr>,
    pub hub: Arc<Hub>,
    pub config: Config,
    listener: TcpListener,
    admin_listener: Option<TcpListener>,
}

impl Server {
    /// Bind `config.listen` and `config.admin_listen` (port 0 picks an
    /// ephemeral one: `addr` and `admin_addr` have the real ones).
    pub async fn bind(config: Config) -> io::Result<Server> {
        let listener = TcpListener::bind(config.listen).await?;
        let addr = listener.local_addr()?;
        let admin_listener = match config.admin_listen {
            Some(at) => Some(TcpListener::bind(at).await?),
            None => None,
        };
        let admin_addr = admin_listener.as_ref().map(TcpListener::local_addr).transpose()?;
        let hub = Hub::new(config.max_rooms, Arc::new(Metrics::new()));
        Ok(Server { addr, admin_addr, hub, config, listener, admin_listener })
    }

    /// The connection keep-alive, set before serving: a test shortens it
    /// to see a silent client go without waiting ten seconds.
    pub fn keep_alive(mut self, keep_alive: crate::hub::KeepAlive) -> Server {
        Arc::get_mut(&mut self.hub).expect("the hub is only shared once the server runs").keep_alive = keep_alive;
        self
    }

    /// Serve until `shutdown` resolves, then finish the open connections.
    /// The admin listener is not part of the shutdown: it answers through
    /// the whole drain and goes with the process - liveness has to hold
    /// for as long as the drain runs, and the metrics matter most then.
    ///
    /// **Every accepted socket gets `TCP_NODELAY`.** The room sends a
    /// small frame sixty times a second and hears one back as often; with
    /// Nagle's algorithm on, the kernel holds each small segment until the
    /// previous one is acknowledged, so on a real link the sixty-a-second
    /// stream leaves in bursts once a round trip - jitter the client
    /// cannot interpolate away, and intents that arrive in clumps and
    /// starve the ticks between. axum does not set it by itself.
    pub async fn run(self, shutdown: impl Future<Output = ()> + Send + 'static) -> io::Result<()> {
        if let Some(admin_listener) = self.admin_listener {
            let router = admin_router(self.hub.clone());
            tokio::spawn(async move {
                if let Err(e) = axum::serve(admin_listener, router).await {
                    warn!("the admin listener stopped: {e}");
                }
            });
        }
        let listener = self.listener.tap_io(|tcp| {
            if let Err(e) = tcp.set_nodelay(true) {
                warn!("TCP_NODELAY refused on an accepted socket: {e}");
            }
        });
        axum::serve(listener, router(self.hub)).with_graceful_shutdown(shutdown).await
    }
}

/// The players' routes: the socket, and nothing else.
fn router(hub: Arc<Hub>) -> Router {
    Router::new().route("/ws", get(ws)).with_state(hub)
}

/// The operator's routes.
fn admin_router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .with_state(hub)
}

/// Liveness: the server answers. It reads the room list on the way, so a
/// hub that has seized up - a lock never let go, a runtime with no worker
/// free - does not answer and the kubelet restarts the process; it never
/// looks at the drain, which a restart would cut short.
async fn health(State(hub): State<Arc<Hub>>) -> Response {
    let rooms = hub.room_count();
    (StatusCode::OK, format!("ok, {rooms} rooms\n")).into_response()
}

/// Readiness: taking new rooms. 503 from the moment the drain starts,
/// which takes the pod out of the Service's endpoints so no new room lands
/// on a server that is leaving; open sockets are served on.
async fn ready(State(hub): State<Arc<Hub>>) -> Response {
    if hub.draining() {
        (StatusCode::SERVICE_UNAVAILABLE, "draining\n").into_response()
    } else {
        (StatusCode::OK, "ok\n").into_response()
    }
}

async fn metrics(State(hub): State<Arc<Hub>>) -> Response {
    let text = hub.metrics.render(hub.counts(), hub.draining());
    ([("content-type", "text/plain; version=0.0.4; charset=utf-8")], text).into_response()
}

async fn ws(ws: WebSocketUpgrade, State(hub): State<Arc<Hub>>) -> Response {
    ws.on_upgrade(move |socket| conn::run(socket, hub))
}
