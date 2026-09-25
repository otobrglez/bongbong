//! The room server's dev surface (`bongbong::devserver::ROOM_TOOLS`,
//! docs/dev-server-design.md): newline-delimited JSON on
//! `127.0.0.1:4849`, the same framing and the same `ToolSpec` rows the
//! game's own dev server speaks, so **one adapter drives both** -
//! `bbmcp` for the window, `bbmcp rooms` for this.
//!
//! **Why the server needs its own.** The game's dev server drives the
//! round in one window. In an online round that window holds a replica
//! it never simulates, so every writing tool there refuses by name and
//! every reading tool describes a picture rather than the truth. The
//! authoritative round lives here, one per room task, and until now
//! nothing could look at it: a co-op bug had to be chased through two
//! real clients and a guess. These tools open a room with no client at
//! all, post a seat's intents the way that seat would, step the round
//! deterministically and read what the server actually did.
//!
//! **Dev-only by construction.** The whole module is behind the
//! `dev-tools` feature, which the release image does not build, and the
//! listener binds loopback only - it is never on the axum router, so
//! nothing here can be reached over `/ws` or any other public route.
//!
//! Discipline is the game's: a socket task only *queues* a request, and
//! the room task answers it between ticks, holding the `Game` the way
//! `before_frame` does. Nothing here touches a world mid-update.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use bongbong::devserver::{REPLY_TIMEOUT, ROOM_TOOLS, ToolSpec};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::hub::Hub;
use crate::room::{Command, RoomParams};

/// The tool table this server answers: the room half of the shared one.
pub const TOOLS: &[ToolSpec] = ROOM_TOOLS;

/// One dev call waiting on a room task, answered at its tick boundary.
pub struct DevRequest {
    pub method: String,
    pub params: Value,
    pub reply: oneshot::Sender<Result<Value, String>>,
}

/// Bind the dev listener and serve it until the process ends. Loopback
/// only: `addr` comes from `--dev-port`, and a bind that fails is logged
/// rather than fatal - a dev tool is never worth taking the server down
/// for.
pub async fn serve(hub: Arc<Hub>, addr: SocketAddr) {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            warn!(%addr, error = %e, "dev server could not bind; tools are unavailable");
            return;
        }
    };
    info!(%addr, tools = TOOLS.len(), "dev server listening (bbmcp rooms)");
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let hub = Arc::clone(&hub);
                tokio::spawn(async move { serve_conn(hub, stream).await });
            }
            Err(e) => {
                warn!(error = %e, "dev server accept failed");
                return;
            }
        }
    }
}

/// One connection: a request per line, a reply per line, until it closes.
async fn serve_conn(hub: Arc<Hub>, stream: TcpStream) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = reply_line(&mut write, &json!({ "id": Value::Null, "error": format!("bad request: {e}") })).await;
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let answer = dispatch(&hub, &method, &params).await;
        let frame = match answer {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(error) => json!({ "id": id, "error": error }),
        };
        if reply_line(&mut write, &frame).await.is_err() {
            return;
        }
    }
}

async fn reply_line(write: &mut tokio::net::tcp::OwnedWriteHalf, frame: &Value) -> std::io::Result<()> {
    write.write_all(format!("{frame}\n").as_bytes()).await?;
    write.flush().await
}

/// **The single dispatch table.** Every row of `TOOLS` is an arm here and
/// every arm is a row (`the_tool_table_and_the_dispatch_agree` holds the
/// two together, the way the game's dev server does).
///
/// Hub-level tools are answered straight off the shared state; anything
/// naming a `code` is forwarded to that room's task, which owns the
/// `Game` and answers between ticks.
pub async fn dispatch(hub: &Arc<Hub>, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "server_status" => Ok(server_status(hub)),
        "rooms" => Ok(rooms(hub)),
        "room_open" => room_open(hub, params).await,
        // Everything else is a room's own business.
        "room" | "room_step" | "room_resume" | "seat_intent" | "room_snapshot" | "room_events"
        | "room_close" => forward(hub, method, params).await,
        other => Err(format!("unknown tool {other:?} - `tools/list` has the set")),
    }
}

fn server_status(hub: &Arc<Hub>) -> Value {
    let counts = hub.counts();
    json!({
        "rooms": hub.room_count(),
        "max_rooms": hub.max_rooms,
        "draining": hub.draining(),
        "protocol_version": bongbong::net::PROTOCOL_VERSION,
        "phases": {
            "waiting": counts.waiting,
            "playing": counts.playing,
            "paused": counts.paused,
            "ended": counts.ended,
        },
        "seats": { "connected": counts.seats_connected, "away": counts.seats_away },
        "metrics": hub.metrics.summary(),
    })
}

fn rooms(hub: &Arc<Hub>) -> Value {
    let rows: Vec<Value> = hub
        .list()
        .into_iter()
        .map(|h| {
            json!({
                "code": h.code,
                "phase": h.stats.phase().name(),
                "seats_connected": h.stats.seats_connected.load(Ordering::Relaxed),
                "seats_away": h.stats.seats_away.load(Ordering::Relaxed),
            })
        })
        .collect();
    json!({ "rooms": rows })
}

/// Open a room with no client and start its round. Two steps, because a
/// room is a task: the hub mints it, then the task itself seats the bots
/// and begins - the same order a hosting client's `Create` then `Start`
/// takes, with the lobby's waiting-for-a-human part left out.
async fn room_open(hub: &Arc<Hub>, params: &Value) -> Result<Value, String> {
    let seats = params.get("seats").and_then(Value::as_u64).unwrap_or(1);
    if !(1..=bongbong::net::MAX_SEATS as u64).contains(&seats) {
        return Err(format!("seats must be 1..={}", bongbong::net::MAX_SEATS));
    }
    let map = params.get("map").and_then(Value::as_str).unwrap_or("default");
    let map_toml = params.get("map_toml").and_then(Value::as_str);
    let mission = params.get("mission").and_then(Value::as_str);
    let seed = params.get("seed").and_then(Value::as_u64);
    let params = RoomParams::for_dev(map, map_toml, mission, seed)?;
    let handle = hub.create_room(params)?;
    let code = handle.code.clone();
    let result = ask(&handle.commands, "room_open", &json!({ "seats": seats })).await;
    match result {
        Ok(mut value) => {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("code".into(), json!(code));
            }
            Ok(value)
        }
        Err(e) => Err(e),
    }
}

async fn forward(hub: &Arc<Hub>, method: &str, params: &Value) -> Result<Value, String> {
    let code = params.get("code").and_then(Value::as_str).ok_or("this tool needs a room `code`")?;
    let handle = hub.find(code)?;
    ask(&handle.commands, method, params).await
}

/// Queue one request on a room task and wait for its answer.
///
/// **Bounded.** A room task that never answers - wedged, starved of a
/// thread, or working through a `room_step` far longer than anyone meant -
/// must not hang the caller: an unbounded wait here shows up as a silent
/// tool call that never returns, which is indistinguishable from the
/// server being broken. `REPLY_TIMEOUT` is the game dev server's own, and
/// the adapter outlasts it deliberately so this message is the one the
/// model reads.
async fn ask(
    commands: &tokio::sync::mpsc::Sender<Command>,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    let (reply, answer) = oneshot::channel();
    let request = DevRequest { method: method.to_string(), params: params.clone(), reply };
    commands.send(Command::Dev(request)).await.map_err(|_| "the room is gone".to_string())?;
    match tokio::time::timeout(REPLY_TIMEOUT, answer).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) => Err("the room closed without answering".to_string()),
        Err(_) => Err(format!(
            "the room did not answer {method:?} within {}s - it is wedged or still working",
            REPLY_TIMEOUT.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;

    fn hub() -> Arc<Hub> {
        Hub::new(8, Arc::new(Metrics::new()))
    }

    /// **The table and the dispatch cannot drift.** A row with no arm
    /// would be advertised to a model and then refused as unknown; an arm
    /// with no row would be unreachable. `TOOLS`'s own discipline, held
    /// here by calling every advertised name and insisting the answer is
    /// anything but "unknown tool".
    #[tokio::test]
    async fn every_advertised_tool_has_an_arm() {
        let hub = hub();
        for spec in TOOLS {
            // No code, so a room tool fails on the lookup rather than
            // doing anything - which is exactly what is being checked.
            let answer = dispatch(&hub, spec.name, &json!({})).await;
            if let Err(e) = &answer {
                assert!(!e.contains("unknown tool"), "{} is advertised but not dispatched: {e}", spec.name);
            }
        }
        let miss = dispatch(&hub, "no_such_tool", &json!({})).await;
        assert!(miss.unwrap_err().contains("unknown tool"));
    }

    /// Every schema the adapter will hand a model has to be JSON it can
    /// parse, or `tools/list` panics at the first call.
    #[test]
    fn every_schema_is_valid_json() {
        for spec in TOOLS {
            let schema: Value = serde_json::from_str(spec.schema)
                .unwrap_or_else(|e| panic!("{}: {e}", spec.name));
            assert_eq!(schema["type"], "object", "{}", spec.name);
        }
    }

    /// A room tool called without a code says so, rather than panicking
    /// or silently answering about some other room.
    #[tokio::test]
    async fn a_room_tool_needs_a_code() {
        let hub = hub();
        let e = dispatch(&hub, "room", &json!({})).await.unwrap_err();
        assert!(e.contains("code"), "{e}");
        let e = dispatch(&hub, "room", &json!({ "code": "ZZZZZ" })).await.unwrap_err();
        assert!(e.contains("no room") || e.contains("code"), "{e}");
    }

    #[tokio::test]
    async fn server_status_describes_an_empty_server() {
        let hub = hub();
        let status = dispatch(&hub, "server_status", &json!({})).await.unwrap();
        assert_eq!(status["rooms"], 0);
        assert_eq!(status["max_rooms"], 8);
        assert_eq!(status["draining"], false);
        assert!(status["metrics"]["tick_p99_us"].is_number(), "{status}");
    }
}
