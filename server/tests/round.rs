//! The protocol's CI check (docs/online-coop-prd.md §4.13): a server on an
//! ephemeral loopback port, two headless clients over WebSocket, a room
//! created and joined by code, a round started and driven, the snapshot
//! stream measured and applied into a client replica whose drawn tanks
//! are held to the wire's, a seat reclaimed after a disconnect.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bongbong::ai::Intent;
use bongbong::level::Mission;
use bongbong::net::apply;
use bongbong::net::client::{ClientEvent, Identity, Phase, RoomClient, RoomSetup};
use bongbong::net::native::NativeTransport;
use bongbong::net::rooms::{RoomCode, RoomsHost, socket_url};
use bongbong::net::codec::{self, Msg};
use bongbong::net::delta::apply_delta;
use bongbong::net::events::WireEvent;
use bongbong::net::wire::{IntentMsg, Lobby, RoundOutcome, Snapshot};
use bongbong::net::{MAX_SEATS, PROTOCOL_VERSION};
use bongbong::simulation::Game;
use bongbong::tank::Dir;
use bongbong::tuning::Tuning;
use bongbong_server::room::{SEATS_PLAYABLE, SNAPSHOT_EVERY, tuning_patch};
use bongbong_server::{Config, Server};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

const WAIT: Duration = Duration::from_secs(5);

async fn start_server() -> (SocketAddr, std::sync::Arc<bongbong_server::hub::Hub>) {
    let config = Config { listen: "127.0.0.1:0".parse().unwrap(), insecure: true, max_rooms: 8 };
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
    assert!(
        code.bytes().all(|b| bongbong::net::rooms::CODE_ALPHABET.contains(&b)),
        "every letter is one the lobby's key grid offers: {code}"
    );
    let welcome = expect_welcome(&mut host).await;
    assert_eq!(welcome.protocol, PROTOCOL_VERSION);
    assert_eq!(welcome.seat, 0);
    assert_eq!(welcome.roster.len(), 1);
    assert!(welcome.map_toml.contains("cells"), "the map travels in the welcome");
    assert_eq!(welcome.snapshot.tick, 0, "a waiting room has no round yet");

    // A second player joins by code; a code that is not one, and a
    // well-formed code no room answers to, are both refused by name.
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
    send(&mut third, &join("third", "tok-third", "CK7QO")).await;
    let refused = expect_lobby_error(&mut third).await;
    assert!(refused.contains("5 letters from"), "names the shape: {refused}");
    drop(third);

    let mut fourth = connect(addr).await;
    // One letter off the room that exists, so it is a code and no room.
    let no_such_room = format!("{}{}", if code.starts_with('C') { 'D' } else { 'C' }, &code[1..]);
    send(&mut fourth, &join("fourth", "tok-fourth", &no_such_room)).await;
    let refused = expect_lobby_error(&mut fourth).await;
    assert!(refused.contains(&no_such_room), "names the code it could not find: {refused}");
    drop(fourth);

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

    // Cadence: snapshots at 30 Hz, the tick at 60 Hz.
    let n = host_stream.deltas.len();
    assert!(n >= 20, "only {n} deltas in {span:?}");
    let (t_first, tick_first) = host_stream.deltas[0];
    let (t_last, tick_last) = host_stream.deltas[n - 1];
    let elapsed = t_last.duration_since(t_first).as_secs_f64();
    let snapshot_hz = (n - 1) as f64 / elapsed;
    let tick_hz = (tick_last - tick_first) as f64 / elapsed;
    eprintln!("measured: {n} deltas over {elapsed:.2} s, snapshots {snapshot_hz:.1} Hz, tick {tick_hz:.1} Hz, {intents_sent} intents sent, {} full snapshots to the host", host_stream.fulls);
    assert!((54.0..=66.0).contains(&snapshot_hz), "snapshots at {snapshot_hz:.1} Hz, wanted 60");
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
    // The baseline sits on the room's snapshot cadence, whatever it is.
    // Spelled with `SNAPSHOT_EVERY` rather than a literal: written as
    // `tick / 3 * 3 == tick` it silently became a one-in-three coin flip
    // the moment the cadence changed.
    assert_eq!(second_stream.baseline.tick as u64 % SNAPSHOT_EVERY, 0, "tick {}", second_stream.baseline.tick);

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

/// A room seats a whole team: every seat up to `SEATS_PLAYABLE` is
/// handed out in order, and the one after it is refused by name.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_room_fills_to_the_seats_it_takes_and_refuses_the_next() {
    assert_eq!(SEATS_PLAYABLE, MAX_SEATS, "a room takes the seats the simulation holds");
    let (addr, _hub) = start_server().await;
    let mut host = connect(addr).await;
    send(&mut host, &create(7)).await;
    let code = expect(&mut host, "the code", |m| match m {
        Msg::Lobby(Lobby::RoomCreated { code }) => Ok(code),
        other => Err(other),
    })
    .await;
    let _ = expect_welcome(&mut host).await;
    let mut seated = vec![host];
    for i in 1..SEATS_PLAYABLE {
        let mut ws = connect(addr).await;
        send(&mut ws, &join(&format!("p{i}"), &format!("tok-p{i}"), &code)).await;
        let w = expect_welcome(&mut ws).await;
        assert_eq!(w.seat as usize, i, "seats are handed out in order");
        assert_eq!(w.roster.len(), i + 1);
        seated.push(ws);
    }
    let mut over = connect(addr).await;
    send(&mut over, &join("over", "tok-over", &code)).await;
    let refused = expect_lobby_error(&mut over).await;
    assert!(refused.contains("full") && refused.contains(&format!("{SEATS_PLAYABLE} seats")), "{refused}");
}

/// Four seats, one round: the room starts with four players, every
/// client is welcomed into it, and every client's replica draws exactly
/// the tanks the wire carries as the stream goes on.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn four_seats_play_one_round_and_every_replica_follows_the_wire() {
    const SEATS: usize = 4;
    let (addr, _hub) = start_server().await;
    let mut host = connect(addr).await;
    send(&mut host, &create(0xF00D)).await;
    let code = expect(&mut host, "the code", |m| match m {
        Msg::Lobby(Lobby::RoomCreated { code }) => Ok(code),
        other => Err(other),
    })
    .await;
    let _ = expect_welcome(&mut host).await;

    let mut clients = vec![host];
    for seat in 1..SEATS as u8 {
        let mut ws = connect(addr).await;
        send(&mut ws, &join(&format!("guest{seat}"), &format!("tok-{seat}"), &code)).await;
        let w = expect_welcome(&mut ws).await;
        assert_eq!(w.seat, seat, "seats are handed out in order");
        send(&mut ws, &Msg::Lobby(Lobby::Ready)).await;
        clients.push(ws);
    }
    // The host asks for the round once the room says everyone else is
    // ready, which is the condition the server checks itself.
    expect(&mut clients[0], "a roster of four, all of them ready", |m| match m {
        Msg::Lobby(Lobby::Roster { seats, .. }) if seats.len() == SEATS && seats.iter().all(|s| s.ready || s.seat == 0) => {
            Ok(())
        }
        other => Err(other),
    })
    .await;
    send(&mut clients[0], &Msg::Lobby(Lobby::Start)).await;

    let span = Duration::from_millis(800);
    let mut following = Vec::new();
    for (seat, mut ws) in clients.into_iter().enumerate() {
        expect(&mut ws, "started", |m| match m {
            Msg::Lobby(Lobby::Started) => Ok(()),
            other => Err(other),
        })
        .await;
        let w = expect_welcome(&mut ws).await;
        assert_eq!(w.seat as usize, seat);
        assert_eq!(w.roster.len(), SEATS, "the whole team travels in the welcome");
        // The round is sized to the team, and every seat is told the same
        // numbers the room played `init` under (docs/online-coop-prd.md
        // §4.11): a client resolves the room's plan, not the map's.
        assert_eq!(w.tuning_json, tuning_patch(SEATS), "seat {seat} got the team's wave plan");
        let knobs = Tuning::DEFAULT.with_json_patch(&w.tuning_json).expect("a patch the build knows every row of");
        assert!(knobs.wave_size_scale > 1.0, "four seats meet a wider wave than one does");
        let ids: Vec<u16> = w.snapshot.tanks.iter().map(|t| t.id).collect();
        assert!(ids.starts_with(&[0, 1, 2, 3]), "a tank per seat leads the roster: {ids:?}");
        let replica = apply::welcome(&w).expect("a replica from the welcome");
        assert_eq!(replica.players.count(), SEATS, "the round started with a seat per client");
        let baseline = w.snapshot;
        following.push(tokio::spawn(async move {
            // Every seat drives for a moment, so the wire carries four
            // tanks that are not where they started.
            let dir = 1 + (seat as u8 % 4);
            for tick in 1..=10u32 {
                send(&mut ws, &Msg::Intent(IntentMsg { tick, move_dir: dir, face: dir, fire: false })).await;
            }
            follow(&mut ws, baseline, Some(replica), span).await
        }));
    }
    for (seat, task) in following.into_iter().enumerate() {
        let stream = task.await.unwrap();
        assert!(stream.deltas.len() >= 5, "seat {seat} saw {} deltas", stream.deltas.len());
        assert_eq!(stream.fulls, 0, "seat {seat} kept up and never needed a full snapshot");
        assert!(stream.compared >= 1, "seat {seat}'s replica was never held to the wire");
        let ids: Vec<u16> = stream.baseline.tanks.iter().map(|t| t.id).collect();
        for tank in 0..SEATS as u16 {
            assert!(ids.contains(&tank), "seat {seat}: tank {tank} left the stream: {ids:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lobby_refusals_are_errors_not_closes() {
    let (addr, _hub) = start_server().await;
    let mut ws = connect(addr).await;
    send(&mut ws, &Msg::Lobby(Lobby::Ready)).await;
    assert_eq!(expect_lobby_error(&mut ws).await, "not in a room");
    send(&mut ws, &join("x", "tok-x", "CZZZZ")).await;
    assert!(expect_lobby_error(&mut ws).await.contains("room code"));
    send(&mut ws, &join("x", "tok-x", "CKKKK")).await;
    assert!(expect_lobby_error(&mut ws).await.contains("no room CKKKK"));
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

// ---------------------------------------------------------------------------
// The client's own transport (docs/online-coop-prd.md §4.6): the same
// round again, but played through `bongbong::net::native` and
// `RoomClient` instead of a hand-driven socket - the path a player's
// machine takes, from the URL the rooms host derives to the deltas
// landing in a replica.

/// What one round through the client's own transport looked like.
struct Played {
    code: String,
    seat: u8,
    /// (arrival, tick) per snapshot the client handed out after the
    /// round's welcome.
    snapshots: Vec<(Instant, u32)>,
    intents: u32,
    /// How many times the replica was held to the wire.
    compared: usize,
}

/// Host a room on `url`, start the round, drive it for `span` and follow
/// the stream. Blocking, the way the game's frame loop is: one poll and
/// one intent per 16 ms, no runtime.
fn play_a_round(url: String, span: Duration) -> Played {
    let mut client = RoomClient::host(
        NativeTransport::connect(&url),
        Identity::new("host", "tok-host"),
        RoomSetup {
            map: "default".into(),
            map_toml: None,
            mission: Mission::Protect,
            seed: Some(0xB0B5),
        },
    );
    let mut events = Vec::new();
    let mut code: Option<String> = None;
    let mut replica: Option<Game> = None;
    let mut played =
        Played { code: String::new(), seat: 0, snapshots: Vec::new(), intents: 0, compared: 0 };
    let (mut asked_to_start, mut in_round) = (false, false);
    let mut round_end: Option<Instant> = None;
    let give_up = Instant::now() + WAIT * 4;
    loop {
        events.clear();
        client.poll(&mut events);
        for event in events.drain(..) {
            match event {
                ClientEvent::Created { code: minted } => code = Some(minted),
                ClientEvent::Started => in_round = true,
                ClientEvent::Welcomed(w) if in_round => {
                    assert_eq!(w.seed, 0xB0B5, "the round runs on the seed the host pinned");
                    let game = apply::welcome(&w).expect("a replica from the welcome");
                    assert_replica_matches(&game, &w.snapshot);
                    replica = Some(game);
                    round_end = Some(Instant::now() + span);
                }
                ClientEvent::Welcomed(_) => {}
                ClientEvent::Snapshot(snapshot) => {
                    let game = replica.as_mut().expect("a snapshot before the welcome");
                    apply::snapshot(game, &snapshot);
                    played.snapshots.push((Instant::now(), snapshot.tick));
                    if played.snapshots.len() % COMPARE_EVERY == 0 {
                        assert_replica_matches(game, &snapshot);
                        played.compared += 1;
                    }
                }
                ClientEvent::Refused(why) => panic!("the room refused: {why}"),
                ClientEvent::Closed(why) => panic!("the socket closed: {why}"),
                ClientEvent::Ended { outcome } => panic!("the round ended early: {outcome:?}"),
                ClientEvent::Roster { .. } | ClientEvent::Said { .. } => {}
            }
        }
        if !asked_to_start && client.phase() == &Phase::Lobby {
            client.start();
            asked_to_start = true;
        }
        if replica.is_some() {
            // A circle, a shot every 90 ticks: the same drive the
            // hand-built client above does.
            let leg = (played.intents / 30) % 4;
            let dir = [Dir::Up, Dir::Right, Dir::Down, Dir::Left][leg as usize];
            let fire = played.intents % 90 == 0;
            client.send_intent(&Intent { move_dir: Some(dir), face: Some(dir), fire, ..Intent::default() });
            played.intents += 1;
        }
        if round_end.is_some_and(|end| Instant::now() >= end) {
            break;
        }
        assert!(Instant::now() < give_up, "the round never got going: {:?}", client.phase());
        std::thread::sleep(Duration::from_millis(16));
    }
    if let Some(game) = &replica {
        assert_replica_matches(game, client.snapshot().expect("a baseline"));
        played.compared += 1;
    }
    played.code = code.expect("a room code");
    played.seat = client.seat().expect("a seat");
    client.close();
    played
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_games_own_transport_hosts_a_round_and_keeps_up_with_it() {
    let (addr, hub) = start_server().await;

    // The URL rule, whole: one server, addressed at its own /ws, whether
    // a room is being made or joined.
    let rooms = RoomsHost::overriding(&format!("ws://{addr}"));
    let url = socket_url(&rooms);
    assert_eq!(url, format!("ws://{addr}/ws"));

    let span = Duration::from_secs(2);
    let played = tokio::task::spawn_blocking(move || play_a_round(url, span))
        .await
        .expect("the client's thread");

    // The code the room minted is one the client parses, and it picks
    // no path: the deployed host is dialled at its own /ws too.
    assert_eq!(played.seat, 0, "the host takes the first seat");
    RoomCode::parse(&played.code).expect("a well-formed code");
    assert_eq!(socket_url(&RoomsHost::deployed()), "wss://rooms.bongbong.io/ws");

    // Cadence: snapshots at 30 Hz, the tick at 60 Hz.
    let n = played.snapshots.len();
    assert!(n >= 25, "only {n} snapshots in {span:?}");
    let (t_first, tick_first) = played.snapshots[0];
    let (t_last, tick_last) = played.snapshots[n - 1];
    let elapsed = t_last.duration_since(t_first).as_secs_f64();
    let snapshot_hz = (n - 1) as f64 / elapsed;
    let tick_hz = (tick_last - tick_first) as f64 / elapsed;
    eprintln!(
        "measured: {n} snapshots over {elapsed:.2} s, snapshots {snapshot_hz:.1} Hz, tick {tick_hz:.1} Hz, {} intents sent, replica checked {} times",
        played.intents, played.compared
    );
    assert!((54.0..=66.0).contains(&snapshot_hz), "snapshots at {snapshot_hz:.1} Hz, wanted 60");
    assert!((54.0..=66.0).contains(&tick_hz), "tick at {tick_hz:.1} Hz, wanted 60");
    assert!(
        played.snapshots.windows(2).all(|w| w[1].1 == w[0].1 + SNAPSHOT_EVERY as u32),
        "a client that keeps up gets every interval, in order"
    );
    assert!(played.intents >= 60, "only {} intents in {span:?}", played.intents);
    assert!(played.compared >= 3, "the replica was held to the wire {} times", played.compared);
    let (_, p99) = hub.metrics.tick_percentiles();
    assert!(p99 < 16_667, "a tick must fit in the tick: p99 {p99} us");
}

// ---------------------------------------------------------------------------
// The end of a round (docs/online-coop-prd.md §4.7): a room ticks its round
// through the end screen and stops the tick before the one a local round
// would restart on, tells every seat how it went, and takes the host's next
// `Start` as a rematch.

/// The fixture the round is ended on; its own header says how it works.
/// `Lobby::Create` carries it whole, so the room needs no file of its own.
const HUNT_MAP: &str = include_str!("../../maps/test/online/hunt-duel.toml");

/// The end screen's length in snapshots, 3 s of `restart_delay` at 20 Hz
/// less the slack of where in the interval the round turned.
const END_SCREEN_SNAPSHOTS: usize = 50;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_round_that_ends_plays_its_end_screen_out_and_comes_back_to_the_lobby() {
    let (addr, hub) = start_server().await;

    // Two seats in a room on the map the first of them wins in a second.
    // No seed is pinned, so the room draws one - and another for the rematch.
    let mut host = connect(addr).await;
    send(
        &mut host,
        &Msg::Lobby(Lobby::Create {
            nick: "host".into(),
            device_token: "tok-host".into(),
            map: "default".into(),
            map_toml: Some(HUNT_MAP.into()),
            mission: Mission::Hunt,
            seed: None,
        }),
    )
    .await;
    let code = expect(&mut host, "the code", |m| match m {
        Msg::Lobby(Lobby::RoomCreated { code }) => Ok(code),
        other => Err(other),
    })
    .await;
    let _ = expect_welcome(&mut host).await;
    let mut guest = connect(addr).await;
    send(&mut guest, &join("guest", "tok-guest", &code)).await;
    let _ = expect_welcome(&mut guest).await;
    ready_then_start(&mut host, &mut guest).await;

    let first = start_of_round(&mut host).await;
    let guest_start = start_of_round(&mut guest).await;
    assert_eq!(guest_start.seed, first.seed, "both seats play the same round");

    // The guest sits the round out; all it has to do is hear how it went,
    // which is a whole round's worth of snapshots away.
    let listening = tokio::spawn(async move {
        let outcome = wait_for_the_end(&mut guest).await;
        (guest, outcome)
    });

    // The host pulls its trigger up the lane and follows the stream until
    // the room says the round is over.
    let played = play_to_the_end(&mut host, first.snapshot.clone()).await;

    assert_eq!(played.ended, Some(RoundOutcome::Won), "the hunt is won by killing the enemy frog");
    let (mut guest, guest_outcome) = tokio::time::timeout(WAIT, listening).await.expect("the guest heard the room").unwrap();
    assert_eq!(guest_outcome, RoundOutcome::Won, "every seat is told, not just the one that was playing");

    // The end screen was played out: the room went on ticking and sending
    // for the whole countdown, and the number it counts down reached zero.
    let over: Vec<&(u32, u8, RoundOutcome)> = played.snapshots.iter().filter(|(_, _, o)| *o != RoundOutcome::Playing).collect();
    assert!(
        over.len() >= END_SCREEN_SNAPSHOTS,
        "only {} snapshots over the end screen, wanted {END_SCREEN_SNAPSHOTS}",
        over.len()
    );
    assert!(over.windows(2).all(|w| w[1].0 > w[0].0), "the room stopped ticking on the end screen");
    assert!(over[0].1 >= 25, "the countdown starts at three seconds, not {}", over[0].1);
    assert_eq!(over[over.len() - 1].1, 0, "the countdown never reached zero");

    // And it stopped there: no second round, no world back at tick 0.
    assert_eq!(played.restarts, 0, "the room started a round nobody asked for");
    assert!(played.snapshots.windows(2).all(|w| w[1].0 > w[0].0), "a tick went backwards: the round started over");
    let (_, metrics) = http_get(addr, "/metrics").await;
    assert!(metrics.contains("bongbong_rooms{phase=\"ended\"} 1"), "{metrics}");

    // The roster that follows the announcement is the one the room screen
    // comes back on: everybody unready again, ready for the host's rematch.
    let seats = expect(&mut host, "the roster after the round", |m| match m {
        Msg::Lobby(Lobby::Roster { seats, .. }) => Ok(seats),
        other => Err(other),
    })
    .await;
    assert_eq!(seats.len(), 2);
    assert!(seats.iter().all(|s| !s.ready && s.connected), "{seats:?}");

    // The rematch: the `Start` the room already takes from a host, on a
    // fresh round and a seed of its own.
    ready_then_start(&mut host, &mut guest).await;
    let again = start_of_round(&mut host).await;
    assert_eq!(again.snapshot.tick, 0, "the rematch starts at the top");
    assert_ne!(again.seed, first.seed, "the rematch replays the round it just finished");
    assert!(
        again.snapshot.events.iter().any(|e| matches!(e, WireEvent::RoundStarted { .. })),
        "the rematch's frame-0 snapshot carries its round start: {:?}",
        again.snapshot.events
    );
    let (_, metrics) = http_get(addr, "/metrics").await;
    assert!(metrics.contains("bongbong_rooms{phase=\"playing\"} 1"), "{metrics}");
    let _ = hub;
}

/// The guest says it is ready and the host starts the round. The room
/// refuses a start while a seat is not ready and the two sockets race, so
/// the start waits for the roster that says the guest is.
async fn ready_then_start(host: &mut Client, guest: &mut Client) {
    send(guest, &Msg::Lobby(Lobby::Ready)).await;
    expect(host, "the guest's ready", |m| match m {
        Msg::Lobby(Lobby::Roster { seats, .. }) if seats.len() == 2 && seats[1].ready => Ok(()),
        other => Err(other),
    })
    .await;
    send(host, &Msg::Lobby(Lobby::Start)).await;
}

/// Read `ws` until the room says the round is over, however many
/// snapshots stand in the way.
async fn wait_for_the_end(ws: &mut Client) -> RoundOutcome {
    let give_up = Instant::now() + Duration::from_secs(20);
    while Instant::now() < give_up {
        let left = give_up.saturating_duration_since(Instant::now());
        let Ok(Some(frame)) = tokio::time::timeout(left, ws.next()).await else { break };
        let Message::Binary(bytes) = frame.expect("a frame") else { continue };
        if let Msg::Lobby(Lobby::Ended { outcome }) = codec::decode(&bytes).expect("a protocol message") {
            return outcome;
        }
    }
    panic!("the room never said the round was over");
}

/// `Started` and the `Welcome` that follows it, for one seat.
async fn start_of_round(ws: &mut Client) -> bongbong::net::wire::Welcome {
    expect(ws, "started", |m| match m {
        Msg::Lobby(Lobby::Started) => Ok(()),
        other => Err(other),
    })
    .await;
    expect_welcome(ws).await
}

/// What the host saw of a round it played to the end.
struct PlayedOut {
    /// (tick, `RoundState::restart`, outcome) per snapshot.
    snapshots: Vec<(u32, u8, RoundOutcome)>,
    /// `RoundStarted` events after the welcome's own: a round the room
    /// began on its own.
    restarts: usize,
    ended: Option<RoundOutcome>,
}

/// Drive `ws`'s seat up the lane, one intent every 16 ms, and follow the
/// snapshot stream until the room says the round is over.
async fn play_to_the_end(ws: &mut Client, baseline: Snapshot) -> PlayedOut {
    let mut played = PlayedOut { snapshots: Vec::new(), restarts: 0, ended: None };
    let mut baseline = baseline;
    let mut tick = 0u32;
    let mut clock = tokio::time::interval(Duration::from_millis(16));
    let give_up = Instant::now() + Duration::from_secs(20);
    while played.ended.is_none() && Instant::now() < give_up {
        tokio::select! {
            _ = clock.tick() => {
                tick += 1;
                // Up the lane, and a shell on the press: a held trigger
                // fires once, so the trigger is pulled.
                send(ws, &Msg::Intent(IntentMsg { tick, move_dir: 0, face: 1, fire: tick % 6 == 0 })).await;
            }
            frame = ws.next() => {
                let Some(Ok(Message::Binary(bytes))) = frame else { break };
                let next = match codec::decode(&bytes).expect("a protocol message") {
                    Msg::Delta(d) => apply_delta(&baseline, &d),
                    Msg::Snapshot(s) => s,
                    Msg::Lobby(Lobby::Ended { outcome }) => {
                        played.ended = Some(outcome);
                        break;
                    }
                    _ => continue,
                };
                played.restarts += next.events.iter().filter(|e| matches!(e, WireEvent::RoundStarted { .. })).count();
                played.snapshots.push((next.tick, next.round.restart, next.round.outcome));
                baseline = next;
            }
        }
    }
    played
}

/// **Pulling the trigger has to put a shell in the air.** Every other
/// test here drives a round and holds the replica to the wire; none of
/// them ever checked that the one input a player cares about most
/// actually reaches `Game::update`. A seat that steers but cannot shoot
/// passes all of them.
///
/// A shell is edge-triggered - `simulation::mod`'s `fire_pressed` fires
/// once per physical press and a held key can never re-arm it - so this
/// taps the way a hand does: pressed for a couple of frames, released,
/// again. Every tap owes a shell.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_tap_of_the_trigger_puts_a_shell_in_the_air() {
    let (addr, _hub) = start_server().await;
    let url = socket_url(&RoomsHost::overriding(&format!("ws://{addr}")));

    let (pulls, fired) = tokio::task::spawn_blocking(move || {
        let mut client = RoomClient::host(
            NativeTransport::connect(&url),
            Identity::new("host", "tok-host"),
            RoomSetup { map: "default".into(), map_toml: None, mission: Mission::Protect, seed: Some(0xB0B5) },
        );
        let mut events = Vec::new();
        let (mut asked, mut in_round) = (false, false);
        let mut round_end: Option<Instant> = None;
        let mut fired = 0usize;
        let mut seat = 0u16;
        let (mut taps, mut pulls) = (0usize, 0usize);
        let give_up = Instant::now() + WAIT * 4;
        loop {
            events.clear();
            client.poll(&mut events);
            for event in events.drain(..) {
                match event {
                    ClientEvent::Started => in_round = true,
                    ClientEvent::Welcomed(w) if in_round => {
                        seat = w.seat as u16;
                        round_end = Some(Instant::now() + Duration::from_secs(2));
                    }
                    ClientEvent::Snapshot(s) => {
                        fired += s
                            .events
                            .iter()
                            .filter(|e| matches!(e, WireEvent::Fired { slot, .. } if *slot == seat))
                            .count();
                    }
                    ClientEvent::Refused(why) => panic!("the room refused: {why}"),
                    ClientEvent::Closed(why) => panic!("the socket closed: {why}"),
                    _ => {}
                }
            }
            if !asked && client.phase() == &Phase::Lobby {
                client.start();
                asked = true;
            }
            if round_end.is_some() {
                // Tap: down for two frames, up for ten, the shape of a
                // hand on the space bar.
                let fire = taps % 12 < 2;
                if fire && taps % 12 == 0 {
                    pulls += 1;
                }
                client.send_intent(&Intent { fire, ..Intent::default() });
                taps += 1;
            }
            if round_end.is_some_and(|end| Instant::now() >= end) {
                break;
            }
            assert!(Instant::now() < give_up, "the round never got going: {:?}", client.phase());
            std::thread::sleep(Duration::from_millis(16));
        }
        client.close();
        (pulls, fired)
    })
    .await
    .expect("the client's thread");

    eprintln!("pulled the trigger {pulls} times, {fired} shells came out");
    assert!(pulls >= 6, "the test never got going: only {pulls} trigger pulls");
    assert!(
        fired * 2 >= pulls,
        "pulled the trigger {pulls} times over two seconds and only {fired} shells came out"
    );
}

/// **The whole client, against the real server.** `OnlineRound::send`
/// paces intents against real time and the server's mailbox applies them
/// one per tick, in order; the two clocks are independent. Every other
/// test drives one side or the other - `Lockstep` and the tests above
/// call `RoomClient::send_intent` directly, and `net::rig`'s room is
/// newest-wins rather than the ordered buffer this one runs. This is the
/// intersection, which is where a lost trigger would live.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_whole_client_taps_the_trigger_and_the_room_answers() {
    use bongbong::net::round::OnlineRound;

    let (addr, _hub) = start_server().await;
    let url = socket_url(&RoomsHost::overriding(&format!("ws://{addr}")));

    let (pulls, fired) = tokio::task::spawn_blocking(move || {
        let client = RoomClient::host(
            NativeTransport::connect(&url),
            Identity::new("host", "tok-host"),
            RoomSetup { map: "default".into(), map_toml: None, mission: Mission::Protect, seed: Some(0xB0B5) },
        );
        let mut round = OnlineRound::new(client, "TEST");
        let frame = Duration::from_millis(16);
        let dt = frame.as_secs_f32();
        // Settle: let the room mint the code, start and welcome.
        let give_up = Instant::now() + WAIT * 4;
        while round.game().is_none() {
            round.frame(&Intent::default(), dt);
            round.start_round();
            assert!(Instant::now() < give_up, "the round never got going");
            std::thread::sleep(frame);
        }
        let (mut pulls, mut fired) = (0usize, 0usize);
        for i in 0..240 {
            let fire = i % 12 < 2;
            if i % 12 == 0 {
                pulls += 1;
            }
            round.frame(&Intent { fire, ..Intent::default() }, dt);
            if let Some(game) = round.game() {
                fired += game
                    .events()
                    .iter()
                    .filter(|e| matches!(e, bongbong::simulation::Event::Fired { slot: 0, .. }))
                    .count();
            }
            std::thread::sleep(frame);
        }
        (pulls, fired)
    })
    .await
    .expect("the client's thread");

    eprintln!("pulled the trigger {pulls} times, {fired} shells came back");
    assert!(pulls >= 15, "the test never got going: {pulls} pulls");
    assert!(
        fired * 2 >= pulls,
        "pulled the trigger {pulls} times and only {fired} shells came back from the room"
    );
}

/// **Co-op is two seats, and the second one has to be able to shoot.**
/// A room of two also runs under `tuning_patch(2)`, which a room of one
/// never does, and the guest's intents land in `Input::seats[1]` rather
/// than `[0]`. Both clients are whole `OnlineRound`s, paced against real
/// time the way the window paces them.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn both_seats_of_a_co_op_room_can_shoot() {
    use bongbong::net::round::OnlineRound;

    let (addr, _hub) = start_server().await;
    let url = socket_url(&RoomsHost::overriding(&format!("ws://{addr}")));

    let counts = tokio::task::spawn_blocking(move || {
        let frame = Duration::from_millis(16);
        let dt = frame.as_secs_f32();
        let host = RoomClient::host(
            NativeTransport::connect(&url),
            Identity::new("host", "tok-host"),
            RoomSetup { map: "default".into(), map_toml: None, mission: Mission::Protect, seed: Some(0xB0B5) },
        );
        let mut host = OnlineRound::new(host, "HOST");
        // Spin until the room has a code to join.
        let give_up = Instant::now() + WAIT * 4;
        let code = loop {
            host.frame(&Intent::default(), dt);
            if let Some(code) = host.code() {
                break code.to_string();
            }
            assert!(Instant::now() < give_up, "the room never minted a code");
            std::thread::sleep(frame);
        };
        let guest = RoomClient::join(
            NativeTransport::connect(&url),
            Identity::new("guest", "tok-guest"),
            code,
        );
        let mut guest = OnlineRound::new(guest, "GUEST");
        // Both in, guest ready, host starts. The wait is on the round
        // *ticking*, not on a replica existing: a seat gets a `Welcome`
        // for the waiting room too, so `game().is_some()` is true well
        // before anything is being simulated.
        let ticking = |r: &OnlineRound<NativeTransport>| r.game().is_some_and(|g| g.frame() > 0);
        let mut readied = false;
        while !ticking(&host) || !ticking(&guest) {
            host.frame(&Intent::default(), dt);
            guest.frame(&Intent::default(), dt);
            if !readied && guest.roster().len() >= 2 {
                guest.ready();
                readied = true;
            }
            // `can_start` is already true for a lone host, so waiting on
            // it alone starts a solo round and the guest is refused the
            // room. The host presses START once the guest is on the
            // roster and ready, which is what the lobby's hand does.
            if host.roster().len() >= 2 && host.can_start() {
                host.start_round();
            }
            assert!(
                Instant::now() < give_up,
                "the round never got going: host roster {:?} seat {:?} host? {} can_start {} note {:?} / guest roster {:?} seat {:?} note {:?}",
                host.roster().iter().map(|s| (s.seat, s.ready)).collect::<Vec<_>>(),
                host.seat(),
                host.is_host(),
                host.can_start(),
                host.note(),
                guest.roster().iter().map(|s| (s.seat, s.ready)).collect::<Vec<_>>(),
                guest.seat(),
                guest.note(),
            );
            std::thread::sleep(frame);
        }
        let seats = (host.seat().expect("host seat"), guest.seat().expect("guest seat"));
        let (mut pulls, mut host_fired, mut guest_fired) = (0usize, 0usize, 0usize);
        let mut guest_by_host = 0usize;
        let count = |round: &OnlineRound<NativeTransport>, slot: usize| {
            round.game().map_or(0, |g| {
                g.events()
                    .iter()
                    .filter(|e| matches!(e, bongbong::simulation::Event::Fired { slot: s, .. } if *s == slot))
                    .count()
            })
        };
        // Ten pulls, half a magazine (`max_shells` 20), so ammo can never
        // be what stops a seat shooting.
        for i in 0..120 {
            let fire = i % 12 < 2;
            if i % 12 == 0 {
                pulls += 1;
            }
            host.frame(&Intent { fire, ..Intent::default() }, dt);
            guest.frame(&Intent { fire, ..Intent::default() }, dt);
            host_fired += count(&host, seats.0 as usize);
            guest_fired += count(&guest, seats.1 as usize);
            guest_by_host += count(&host, seats.1 as usize);
            std::thread::sleep(frame);
        }
        // Ammo, so a magazine running dry can never read as a lost
        // trigger: the two chassis carry different ones.
        let ammo: Vec<String> = host.game().map_or_else(Vec::new, |g| {
            g.tank_snapshots()
                .iter()
                .filter(|t| t.is_player)
                .map(|t| format!("seat {} has {} left", t.slot, t.shells_ammo))
                .collect()
        });
        (pulls, seats, host_fired, guest_fired, guest_by_host, ammo)
    })
    .await
    .expect("the clients' thread");

    let (pulls, (host_seat, guest_seat), host_fired, guest_fired, guest_by_host, ammo) = counts;
    eprintln!(
        "{pulls} pulls each: seat {host_seat} (host) fired {host_fired}, seat {guest_seat} (guest) fired {guest_fired} \
         (the host saw {guest_by_host} of the guest's); ammo left: {ammo:?}"
    );
    assert!(pulls >= 8, "the test never got going: {pulls} pulls");
    assert_eq!(host_fired, pulls, "the host pulled {pulls} times");
    assert_eq!(guest_fired, pulls, "the guest pulled {pulls} times");
    // Both seats' shells are in the same round, so one observer has to
    // see them all: a seat firing into its own replica only would pass
    // the two counts above.
    assert_eq!(guest_by_host, pulls, "the host saw only {guest_by_host} of the guest's {pulls} shells");
}

/// Read `ws` until the server closes it, however much stands in the way.
async fn closed_by_server(ws: &mut Client) {
    let give_up = Instant::now() + WAIT;
    while Instant::now() < give_up {
        let left = give_up.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, ws.next()).await {
            Ok(None | Some(Ok(Message::Close(_))) | Some(Err(_))) => return,
            Ok(Some(Ok(_))) => continue,
            Err(_) => break,
        }
    }
    panic!("the server kept the socket open");
}

/// A drain waits for a round being played and nothing else: the waiting
/// room goes at once, with its host told why, the round in progress plays
/// on, and once its last seat drops the server has nothing left to wait for.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drain_closes_idle_rooms_at_once_and_waits_only_for_a_round_in_play() {
    let (_addr, hub) = start_server().await;
    let addr = _addr;
    let mut idle = connect(addr).await;
    send(&mut idle, &create(1)).await;
    let _ = expect_welcome(&mut idle).await;
    let mut playing = connect(addr).await;
    send(&mut playing, &create(2)).await;
    let _ = expect_welcome(&mut playing).await;
    send(&mut playing, &Msg::Lobby(Lobby::Start)).await;
    let _ = start_of_round(&mut playing).await;
    assert_eq!(hub.room_count(), 2);

    hub.begin_drain();
    let why = expect_lobby_error(&mut idle).await;
    assert!(why.contains("restarting"), "{why}");
    closed_by_server(&mut idle).await;
    let give_up = Instant::now() + WAIT;
    while hub.room_count() > 1 && Instant::now() < give_up {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(hub.room_count(), 1, "the waiting room went though its host was still connected");
    let tick = expect(&mut playing, "a snapshot after the drain began", |m| match m {
        Msg::Snapshot(s) => Ok(s.tick),
        Msg::Delta(d) => Ok(d.tick),
        other => Err(other),
    })
    .await;
    assert!(tick > 0, "the round in play keeps ticking");

    drop(playing);
    tokio::time::timeout(WAIT, hub.drained()).await.expect("the paused round is not waited for");
}
