//! The protocol's CI check (docs/online-coop-prd.md §4.13): a server on an
//! ephemeral loopback port, two headless clients over WebSocket, a room
//! created and joined by code, a round started and driven, the snapshot
//! stream measured and applied into a client replica whose drawn tanks
//! are held to the wire's, a seat reclaimed after a disconnect.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bongbong::level::Mission;
use bongbong::net::apply;
use bongbong::net::codec::{self, Msg};
use bongbong::net::delta::apply_delta;
use bongbong::net::events::WireEvent;
use bongbong::net::wire::{IntentMsg, Lobby, Snapshot};
use bongbong::net::{MAX_SEATS, PROTOCOL_VERSION};
use bongbong::simulation::Game;
use bongbong_server::room::{SEATS_PLAYABLE, SNAPSHOT_EVERY};
use bongbong_server::{Config, Server};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

const WAIT: Duration = Duration::from_secs(5);

async fn start_server() -> (SocketAddr, std::sync::Arc<bongbong_server::hub::Hub>) {
    let config = Config { listen: "127.0.0.1:0".parse().unwrap(), pod: 'A', insecure: true, max_rooms: 8 };
    let server = Server::bind(config).await.expect("bind an ephemeral port");
    let addr = server.addr;
    let hub = server.hub.clone();
    tokio::spawn(server.run(std::future::pending()));
    (addr, hub)
}

async fn connect(addr: SocketAddr) -> Client {
    let (ws, _) = connect_async(format!("ws://{addr}/ws")).await.expect("connect");
    ws
}

async fn send(ws: &mut Client, msg: &Msg) {
    ws.send(Message::Binary(codec::encode(msg).into())).await.expect("send");
}

async fn next(ws: &mut Client) -> Msg {
    loop {
        let frame = tokio::time::timeout(WAIT, ws.next()).await.expect("a message within the wait").expect("socket open");
        match frame.expect("a frame") {
            Message::Binary(bytes) => return codec::decode(&bytes).expect("a protocol message"),
            Message::Close(_) => panic!("the server closed the socket"),
            _ => continue,
        }
    }
}

/// The first message `pick` accepts, skipping at most 32 others.
async fn expect<T>(ws: &mut Client, what: &str, pick: impl Fn(Msg) -> Result<T, Msg>) -> T {
    for _ in 0..32 {
        match pick(next(ws).await) {
            Ok(t) => return t,
            Err(other) => eprintln!("skipping {} while waiting for {what}", describe(&other)),
        }
    }
    panic!("no {what} within 32 messages");
}

fn describe(msg: &Msg) -> String {
    match msg {
        Msg::Lobby(l) => format!("lobby {}", serde_json_type(l)),
        Msg::Welcome(w) => format!("welcome seat {}", w.seat),
        Msg::Snapshot(s) => format!("snapshot tick {}", s.tick),
        Msg::Delta(d) => format!("delta tick {}", d.tick),
        Msg::Intent(_) => "intent".into(),
    }
}

fn serde_json_type(l: &Lobby) -> String {
    match l {
        Lobby::Error { message } => format!("error {message:?}"),
        Lobby::RoomCreated { code } => format!("room_created {code}"),
        Lobby::Roster { seats, .. } => format!("roster of {}", seats.len()),
        Lobby::Started => "started".into(),
        Lobby::Said { .. } => "said".into(),
        other => format!("{other:?}"),
    }
}

async fn expect_lobby_error(ws: &mut Client) -> String {
    expect(ws, "an error", |m| match m {
        Msg::Lobby(Lobby::Error { message }) => Ok(message),
        other => Err(other),
    })
    .await
}

async fn expect_welcome(ws: &mut Client) -> bongbong::net::wire::Welcome {
    expect(ws, "a welcome", |m| match m {
        Msg::Welcome(w) => Ok(w),
        other => Err(other),
    })
    .await
}

async fn http_get(addr: SocketAddr, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let status: u16 = raw.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

fn create(seed: u64) -> Msg {
    Msg::Lobby(Lobby::Create {
        nick: "host".into(),
        device_token: "tok-host".into(),
        map: "default".into(),
        map_toml: None,
        mission: Mission::Protect,
        seed: Some(seed),
    })
}

fn join(nick: &str, token: &str, code: &str) -> Msg {
    Msg::Lobby(Lobby::Join { nick: nick.into(), device_token: token.into(), code: code.into() })
}

/// What one client saw of the snapshot stream.
struct Stream {
    /// (arrival, tick) per delta.
    deltas: Vec<(Instant, u32)>,
    fulls: usize,
    baseline: Snapshot,
    events: Vec<WireEvent>,
    /// The client's replica, built from the welcome and stepped by every
    /// snapshot; `None` for a client that only counts.
    replica: Option<Game>,
    /// How many times the replica's drawn tanks were held to the wire's.
    compared: usize,
}

/// Every snapshot lands in the replica; every `COMPARE_EVERY`th one, the
/// replica's drawn tanks are checked against it.
const COMPARE_EVERY: usize = 10;

/// The replica draws exactly the tanks the snapshot lists, at the
/// snapshot's quantised positions.
fn assert_replica_matches(replica: &Game, snap: &Snapshot) {
    let drawn: Vec<(usize, i32, i32)> = replica.drawable_state().tanks.iter().map(|t| (t.slot, t.x, t.y)).collect();
    let wire: Vec<(usize, i32, i32)> = snap.tanks.iter().map(|t| (t.id as usize, t.x as i32, t.y as i32)).collect();
    assert_eq!(drawn, wire, "replica tanks against the snapshot at tick {}", snap.tick);
    assert_eq!(replica.frame(), snap.tick as u64);
}

impl Stream {
    fn new(baseline: Snapshot, replica: Option<Game>) -> Stream {
        if let Some(r) = &replica {
            assert_replica_matches(r, &baseline);
        }
        Stream { deltas: Vec::new(), fulls: 0, baseline, events: Vec::new(), replica, compared: 0 }
    }

    /// The baseline moved on to `snap`: the replica follows, and is
    /// checked every `COMPARE_EVERY`th time.
    fn land(&mut self, snap: Snapshot) {
        if let Some(r) = &mut self.replica {
            apply::snapshot(r, &snap);
            if (self.deltas.len() + self.fulls) % COMPARE_EVERY == 0 {
                assert_replica_matches(r, &snap);
                self.compared += 1;
            }
        }
        self.baseline = snap;
    }
}

/// Read `ws` for `span`, applying every delta onto the baseline and into
/// the replica; a client that keeps up sees only deltas.
async fn follow(ws: &mut Client, baseline: Snapshot, replica: Option<Game>, span: Duration) -> Stream {
    let mut stream = Stream::new(baseline, replica);
    let end = Instant::now() + span;
    while Instant::now() < end {
        let left = end.saturating_duration_since(Instant::now());
        let Ok(Some(frame)) = tokio::time::timeout(left, ws.next()).await else { break };
        let Message::Binary(bytes) = frame.expect("a frame") else { continue };
        match codec::decode(&bytes).expect("a protocol message") {
            Msg::Delta(d) => {
                assert_eq!(d.tick, stream.baseline.tick + SNAPSHOT_EVERY as u32, "every third tick, none skipped");
                let next = apply_delta(&stream.baseline, &d);
                assert_eq!(next.tick, d.tick);
                assert!(next.tanks.iter().any(|t| t.id == 0) && next.tanks.iter().any(|t| t.id == 1), "both players' tanks persist");
                stream.events.extend(next.events.iter().cloned());
                stream.deltas.push((Instant::now(), d.tick));
                stream.land(next);
            }
            Msg::Snapshot(s) => {
                stream.fulls += 1;
                stream.land(s);
            }
            other => eprintln!("follow: ignoring {}", describe(&other)),
        }
    }
    if let Some(r) = &stream.replica {
        assert_replica_matches(r, &stream.baseline);
        stream.compared += 1;
    }
    stream
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_clients_play_a_round_and_a_seat_survives_a_reconnect() {
    let (addr, hub) = start_server().await;

    // The host creates the room.
    let mut host = connect(addr).await;
    send(&mut host, &create(0xB0B5)).await;
    let code = expect(&mut host, "the code", |m| match m {
        Msg::Lobby(Lobby::RoomCreated { code }) => Ok(code),
        other => Err(other),
    })
    .await;
    assert_eq!(code.len(), 5);
    assert!(code.starts_with('A'), "the pod letter leads: {code}");
    let welcome = expect_welcome(&mut host).await;
    assert_eq!(welcome.protocol, PROTOCOL_VERSION);
    assert_eq!(welcome.seat, 0);
    assert_eq!(welcome.roster.len(), 1);
    assert!(welcome.map_toml.contains("cells"), "the map travels in the welcome");
    assert_eq!(welcome.snapshot.tick, 0, "a waiting room has no round yet");

    // A second player joins by code; a wrong pod and a third seat are refused.
    let mut second = connect(addr).await;
    send(&mut second, &join("second", "tok-second", &code)).await;
    let welcome2 = expect_welcome(&mut second).await;
    assert_eq!(welcome2.seat, 1);
    assert_eq!(welcome2.roster.len(), 2);
    let roster = expect(&mut host, "the roster", |m| match m {
        Msg::Lobby(Lobby::Roster { host, seats }) if seats.len() == 2 => Ok((host, seats)),
        other => Err(other),
    })
    .await;
    assert_eq!(roster.0, 0);
    assert_eq!(roster.1[1].nick, "second");

    let mut third = connect(addr).await;
    let other_pod = format!("B{}", &code[1..]);
    send(&mut third, &join("third", "tok-third", &other_pod)).await;
    let refused = expect_lobby_error(&mut third).await;
    assert!(refused.contains("pod B") && refused.contains("pod A"), "names the mismatch: {refused}");
    send(&mut third, &join("third", "tok-third", &code)).await;
    let refused = expect_lobby_error(&mut third).await;
    assert!(refused.contains(&format!("{SEATS_PLAYABLE} seats")), "{refused}");
    drop(third);

    // Ready, start: both get Started and a fresh Welcome with the round.
    send(&mut second, &Msg::Lobby(Lobby::Ready)).await;
    send(&mut host, &Msg::Lobby(Lobby::Start)).await;
    let (mut host_baseline, mut second_baseline): (Option<Snapshot>, Option<Snapshot>) = (None, None);
    let mut host_replica: Option<Game> = None;
    for (ws, seat) in [(&mut host, 0u8), (&mut second, 1u8)] {
        expect(ws, "started", |m| match m {
            Msg::Lobby(Lobby::Started) => Ok(()),
            other => Err(other),
        })
        .await;
        let w = expect_welcome(ws).await;
        assert_eq!(w.seat, seat);
        assert_eq!(w.seed, 0xB0B5);
        assert_eq!(w.snapshot.tick, 0);
        let ids: Vec<u16> = w.snapshot.tanks.iter().map(|t| t.id).collect();
        assert!(ids.starts_with(&[0, 1]), "both players' tanks lead the roster: {ids:?}");
        assert!(
            w.snapshot.events.iter().any(|e| matches!(e, WireEvent::RoundStarted { seed: 0xB0B5, .. })),
            "the round start travels in the welcome's frame-0 snapshot: {:?}",
            w.snapshot.events
        );
        assert!(w.enemy_count.is_none(), "the room pins no enemy count; the replica rolls the server's");
        assert!(w.dead_cells.is_empty(), "a fresh field has no holes");
        assert!(w.roster.iter().all(|s| s.chassis > 0 || s.seat == 0), "the roster carries the rolled chassis: {:?}", w.roster);
        if seat == 0 {
            host_replica = Some(apply::welcome(&w).expect("a replica from the welcome"));
            host_baseline = Some(w.snapshot);
        } else {
            second_baseline = Some(w.snapshot);
        }
    }
    let host_baseline = host_baseline.take().unwrap();
    let second_baseline = second_baseline.take().unwrap();
    let host_replica = host_replica.take().unwrap();
    assert_eq!(host_replica.round_seed(), 0xB0B5, "the replica ran init on the room's seed");
    let start_x = second_baseline.tanks[1].x;
    let start_y = second_baseline.tanks[1].y;

    // The second player drives a circle for three seconds while the host
    // follows the stream.
    let span = Duration::from_secs(3);
    let driver = tokio::spawn(async move {
        let mut ws = second;
        let mut tick = 0u32;
        let mut clock = tokio::time::interval(Duration::from_millis(16));
        let end = Instant::now() + span;
        let mut seen = Stream::new(second_baseline, None);
        while Instant::now() < end {
            tokio::select! {
                _ = clock.tick() => {
                    tick += 1;
                    let leg = (tick / 30) % 4;
                    let move_dir = [1u8, 4, 2, 3][leg as usize];
                    send(&mut ws, &Msg::Intent(IntentMsg { tick, move_dir, face: move_dir, fire: tick % 90 == 0 })).await;
                }
                frame = ws.next() => {
                    let Some(Ok(Message::Binary(bytes))) = frame else { break };
                    match codec::decode(&bytes).unwrap() {
                        Msg::Delta(d) => {
                            seen.baseline = apply_delta(&seen.baseline, &d);
                            seen.deltas.push((Instant::now(), d.tick));
                        }
                        Msg::Snapshot(s) => { seen.fulls += 1; seen.baseline = s; }
                        _ => {}
                    }
                }
            }
        }
        (ws, seen, tick)
    });
    let host_stream = follow(&mut host, host_baseline, Some(host_replica), span).await;
    let (second, second_stream, intents_sent) = driver.await.unwrap();

    // Cadence: snapshots at 20 Hz, the tick at 60 Hz.
    let n = host_stream.deltas.len();
    assert!(n >= 20, "only {n} deltas in {span:?}");
    let (t_first, tick_first) = host_stream.deltas[0];
    let (t_last, tick_last) = host_stream.deltas[n - 1];
    let elapsed = t_last.duration_since(t_first).as_secs_f64();
    let snapshot_hz = (n - 1) as f64 / elapsed;
    let tick_hz = (tick_last - tick_first) as f64 / elapsed;
    eprintln!("measured: {n} deltas over {elapsed:.2} s, snapshots {snapshot_hz:.1} Hz, tick {tick_hz:.1} Hz, {intents_sent} intents sent, {} full snapshots to the host", host_stream.fulls);
    assert!((17.0..=23.0).contains(&snapshot_hz), "snapshots at {snapshot_hz:.1} Hz, wanted 20");
    assert!((54.0..=66.0).contains(&tick_hz), "tick at {tick_hz:.1} Hz, wanted 60");
    assert_eq!(host_stream.fulls, 0, "a client that keeps up never needs a full snapshot");
    assert!(host_stream.compared >= 5, "the replica was held to the wire {} times", host_stream.compared);
    eprintln!("replica checked against the wire {} times", host_stream.compared);
    assert!(
        !host_stream.events.iter().any(|e| matches!(e, WireEvent::RoundStarted { .. })),
        "no delta repeats the round start (a replica would init again)"
    );
    let (p50, p99) = hub.metrics.tick_percentiles();
    eprintln!("tick p50 {p50} us, p99 {p99} us");
    assert!(p99 < 16_667, "a tick must fit in the tick: p99 {p99} us");

    // The driven tank moved and the second client's ack was carried.
    let tank1 = host_stream.baseline.tanks.iter().find(|t| t.id == 1).expect("the second player's tank");
    assert!(tank1.x != start_x || tank1.y != start_y, "the driven tank moved");
    assert!(host_stream.baseline.acked[1] > 0, "the seat's newest intent tick is acked: {:?}", &host_stream.baseline.acked[..2]);
    assert_eq!(host_stream.baseline.acked[2..], [0; MAX_SEATS - 2]);
    assert_eq!(second_stream.baseline.tick / 3 * 3, second_stream.baseline.tick);

    // The second player drops and comes back with the same token: same
    // seat, a fresh welcome, then the stream resumes with a full snapshot.
    drop(second);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut back = connect(addr).await;
    send(&mut back, &join("second again", "tok-second", &code)).await;
    let again = expect_welcome(&mut back).await;
    assert_eq!(again.seat, 1, "the token reclaims the seat");
    assert_eq!(again.seed, 0xB0B5);
    assert!(again.snapshot.tick > tick_last, "the round went on meanwhile");
    assert!(again.snapshot.tanks.len() >= 2, "the welcome is cut from the live world");
    assert!(again.dead_cells.windows(2).all(|w| w[0] < w[1]), "dead cells are sorted, each once: {:?}", again.dead_cells);
    // A late joiner builds the round as it stands now and follows from there.
    let late_replica = apply::welcome(&again).expect("a replica from the rejoin welcome");
    let resumed = follow(&mut back, again.snapshot.clone(), Some(late_replica), Duration::from_millis(500)).await;
    assert_eq!(resumed.fulls, 1, "a rejoin gets one full snapshot to reset its baseline, then deltas");
    assert!(resumed.deltas.len() >= 5, "{} deltas after the rejoin", resumed.deltas.len());
    assert!(resumed.compared >= 1);
    assert_eq!(hub.metrics.reconnects_total.load(std::sync::atomic::Ordering::Relaxed), 1);
    let roster = expect(&mut host, "the roster after the rejoin", |m| match m {
        Msg::Lobby(Lobby::Roster { seats, .. }) if seats.len() == 2 && seats[1].connected => Ok(seats),
        other => Err(other),
    })
    .await;
    assert_eq!(roster[1].nick, "second", "the seat keeps its name");

    // The HTTP side: health, metrics, and the drain.
    let (status, body) = http_get(addr, "/health").await;
    assert_eq!((status, body.as_str()), (200, "ok\n"));
    let (status, metrics) = http_get(addr, "/metrics").await;
    assert_eq!(status, 200);
    assert!(metrics.contains("bongbong_rooms{phase=\"playing\"} 1"), "{metrics}");
    assert!(metrics.contains("bongbong_seats{state=\"connected\"} 2"), "{metrics}");
    assert!(metrics.contains("bongbong_reconnects_total 1"), "{metrics}");
    hub.begin_drain();
    let (status, _) = http_get(addr, "/health").await;
    assert_eq!(status, 503);
    let mut late = connect(addr).await;
    send(&mut late, &create(1)).await;
    let refused = expect_lobby_error(&mut late).await;
    assert!(refused.contains("draining"), "{refused}");
    let (_, metrics) = http_get(addr, "/metrics").await;
    assert!(metrics.contains("bongbong_draining 1"), "{metrics}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lobby_refusals_are_errors_not_closes() {
    let (addr, _hub) = start_server().await;
    let mut ws = connect(addr).await;
    send(&mut ws, &Msg::Lobby(Lobby::Ready)).await;
    assert_eq!(expect_lobby_error(&mut ws).await, "not in a room");
    send(&mut ws, &join("x", "tok-x", "AZZZZ")).await;
    assert!(expect_lobby_error(&mut ws).await.contains("room code"));
    send(&mut ws, &join("x", "tok-x", "AKKKK")).await;
    assert!(expect_lobby_error(&mut ws).await.contains("no room AKKKK"));
    ws.send(Message::Binary(vec![9, 9].into())).await.unwrap();
    assert!(expect_lobby_error(&mut ws).await.contains("unknown message kind"));
    // A bare JSON text frame is a lobby message too.
    ws.send(Message::Text(r#"{"type":"start"}"#.into())).await.unwrap();
    assert_eq!(expect_lobby_error(&mut ws).await, "not in a room");
    send(&mut ws, &create(3)).await;
    let _code = expect(&mut ws, "the code", |m| match m {
        Msg::Lobby(Lobby::RoomCreated { code }) => Ok(code),
        other => Err(other),
    })
    .await;
    let _ = expect_welcome(&mut ws).await;
    send(&mut ws, &Msg::Lobby(Lobby::Kick { seat: 0 })).await;
    assert!(expect_lobby_error(&mut ws).await.contains("themself"));
    send(&mut ws, &Msg::Lobby(Lobby::Chat { text: "gg".into() })).await;
    let said = expect(&mut ws, "the chat", |m| match m {
        Msg::Lobby(Lobby::Said { seat, text }) => Ok((seat, text)),
        other => Err(other),
    })
    .await;
    assert_eq!(said, (0, "gg".into()));
    send(&mut ws, &Msg::Lobby(Lobby::Leave)).await;
    assert!(expect_lobby_error(&mut ws).await.contains("left"));
}
