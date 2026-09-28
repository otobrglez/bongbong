//! The impairment proxy: a TCP relay between the clients and the room
//! server that delays, bunches and holds bytes the way a real link does
//! (`link::Schedule`), and reads the WebSocket stream going through it
//! back into protocol messages (`wstap`) with the instant each one reached
//! the proxy and the instant it left.
//!
//! Per direction there are two tasks: a reader that stamps each chunk as
//! it arrives and schedules its delivery, and a writer that sleeps until
//! that delivery, writes the chunk and hands it to the tap. Every socket
//! has `TCP_NODELAY`, so the proxy adds no coalescing of its own - Nagle is
//! the model's switch, not the kernel's.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use bongbong::net::MAX_SEATS;
use bongbong::net::codec::{self, Msg};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::link::{Impairment, Schedule};
use crate::wstap::{OP_BINARY, WsMessage, WsTap};

/// The process clock every stamp in a run is read on: milliseconds since
/// one `Instant`, shared by the proxy, the clients and the report.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub epoch: Instant,
}

impl Clock {
    pub fn new() -> Clock {
        Clock { epoch: Instant::now() }
    }

    pub fn now_ms(&self) -> f64 {
        self.ms(Instant::now())
    }

    pub fn ms(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.epoch).as_secs_f64() * 1000.0
    }

    pub fn instant(&self, ms: f64) -> Instant {
        self.epoch + Duration::from_secs_f64(ms.max(0.0) / 1000.0)
    }
}

impl Default for Clock {
    fn default() -> Clock {
        Clock::new()
    }
}

/// An intent as it crossed the proxy toward the server.
#[derive(Clone, Copy, Debug)]
pub struct IntentRec {
    pub ingress_ms: f64,
    pub egress_ms: f64,
    pub tick: u32,
    pub owned: bool,
    pub fire: bool,
}

/// A snapshot or delta as it crossed the proxy toward a client.
#[derive(Clone, Copy, Debug)]
pub struct SnapRec {
    pub ingress_ms: f64,
    pub egress_ms: f64,
    pub tick: u32,
    pub server_ms: u32,
    pub acked: [u32; MAX_SEATS],
    pub full: bool,
    pub bytes: usize,
}

/// Everything the tap read off one connection.
#[derive(Clone, Debug, Default)]
pub struct ConnTap {
    /// The seat the room's `Welcome` gave this connection.
    pub seat: Option<u8>,
    pub intents: Vec<IntentRec>,
    pub snapshots: Vec<SnapRec>,
    /// Every chunk's delivery and size, client to server.
    pub up_chunks: Vec<(f64, usize)>,
    /// Every chunk's delivery and size, server to client.
    pub down_chunks: Vec<(f64, usize)>,
    /// Chunks the model held for a retransmit, each way.
    pub up_lost: u32,
    pub down_lost: u32,
    /// A direction whose frames stopped parsing.
    pub tap_failed: Option<String>,
}

/// Every connection the proxy has carried, in accept order.
#[derive(Clone, Debug, Default)]
pub struct TapLog {
    pub conns: Vec<ConnTap>,
}

pub type SharedLog = Arc<Mutex<TapLog>>;

/// A running proxy.
pub struct Proxy {
    pub addr: SocketAddr,
    pub log: SharedLog,
}

/// Which way a direction's bytes go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Way {
    Up,
    Down,
}

/// Bind `127.0.0.1:0` and relay every connection to `upstream` under
/// `link`. The accept loop runs until the runtime stops.
pub async fn start(upstream: SocketAddr, link: Impairment, seed: u64, clock: Clock) -> std::io::Result<Proxy> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let log: SharedLog = Arc::new(Mutex::new(TapLog::default()));
    let accept_log = log.clone();
    tokio::spawn(async move {
        let mut index = 0u64;
        while let Ok((client, _)) = listener.accept().await {
            let conn = {
                let mut log = accept_log.lock().unwrap_or_else(PoisonError::into_inner);
                log.conns.push(ConnTap::default());
                log.conns.len() - 1
            };
            let log = accept_log.clone();
            let conn_seed = seed ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            index += 1;
            tokio::spawn(async move {
                let Ok(server) = TcpStream::connect(upstream).await else { return };
                let _ = client.set_nodelay(true);
                let _ = server.set_nodelay(true);
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                relay(client_read, server_write, Way::Up, Schedule::new(link, conn_seed), log.clone(), conn, clock);
                relay(server_read, client_write, Way::Down, Schedule::new(link, conn_seed ^ 0xD0), log, conn, clock);
            });
        }
    });
    Ok(Proxy { addr, log })
}

/// One direction: read, schedule, write, tap.
fn relay(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    way: Way,
    mut schedule: Schedule,
    log: SharedLog,
    conn: usize,
    clock: Clock,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<(Vec<u8>, f64, f64)>();
    let reader_log = log.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match from.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let arrival = clock.now_ms();
            let delivery = schedule.deliver(arrival);
            if delivery.lost {
                let mut log = reader_log.lock().unwrap_or_else(PoisonError::into_inner);
                let c = &mut log.conns[conn];
                match way {
                    Way::Up => c.up_lost += 1,
                    Way::Down => c.down_lost += 1,
                }
            }
            if tx.send((buf[..n].to_vec(), arrival, delivery.at_ms)).is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let mut tap = WsTap::new();
        let mut messages: Vec<WsMessage> = Vec::new();
        while let Some((bytes, arrival, at)) = rx.recv().await {
            // A delivery already due goes out at once: the timer wheel's
            // millisecond would otherwise be added to every chunk of a
            // clean link.
            if clock.now_ms() < at {
                tokio::time::sleep_until(tokio::time::Instant::from_std(clock.instant(at))).await;
            }
            if to.write_all(&bytes).await.is_err() {
                break;
            }
            let egress = clock.now_ms();
            messages.clear();
            tap.feed(&bytes, arrival, egress, &mut messages);
            record(&log, conn, way, &bytes, egress, &messages, &tap);
        }
        let _ = to.shutdown().await;
    });
}

/// Put one delivered chunk and the messages it completed on the log.
fn record(log: &SharedLog, conn: usize, way: Way, bytes: &[u8], egress: f64, messages: &[WsMessage], tap: &WsTap) {
    let mut log = log.lock().unwrap_or_else(PoisonError::into_inner);
    let c = &mut log.conns[conn];
    match way {
        Way::Up => c.up_chunks.push((egress, bytes.len())),
        Way::Down => c.down_chunks.push((egress, bytes.len())),
    }
    if let Some(e) = tap.failed()
        && c.tap_failed.is_none()
    {
        c.tap_failed = Some(format!("{way:?}: {e:?}"));
    }
    for m in messages.iter().filter(|m| m.opcode == OP_BINARY) {
        let Ok(msg) = codec::decode(&m.payload) else { continue };
        match msg {
            Msg::Intent(i) => c.intents.push(IntentRec {
                ingress_ms: m.ingress_ms,
                egress_ms: m.egress_ms,
                tick: i.tick,
                owned: i.owned,
                fire: i.fire,
            }),
            Msg::Snapshot(s) => c.snapshots.push(SnapRec {
                ingress_ms: m.ingress_ms,
                egress_ms: m.egress_ms,
                tick: s.tick,
                server_ms: s.server_ms,
                acked: s.acked,
                full: true,
                bytes: m.payload.len(),
            }),
            Msg::Delta(d) => c.snapshots.push(SnapRec {
                ingress_ms: m.ingress_ms,
                egress_ms: m.egress_ms,
                tick: d.tick,
                server_ms: d.server_ms,
                acked: d.acked,
                full: false,
                bytes: m.payload.len(),
            }),
            Msg::Welcome(w) => c.seat = Some(w.seat),
            _ => {}
        }
    }
}
