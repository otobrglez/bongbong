//! The client's half of a room, over any `Transport`
//! (docs/online-coop-prd.md §4.3, §4.10): say "host a room" or "join
//! AK7QX" and read back the code, the roster, the `Welcome` and then the
//! snapshots. A caller drives it with one `poll` per frame and never
//! touches the protocol.
//!
//! What this layer owns is the lobby's state machine and the delta
//! chain. A `SnapshotDelta` is applied onto the last snapshot here
//! (`net::delta::apply_delta`), so a caller only ever sees whole
//! `Snapshot`s, in tick order, starting with the one inside the
//! `Welcome`. What it deliberately does not own is the `Game`: turning a
//! welcome into a replica and a snapshot into a world is `net::apply`'s
//! job, and whose `Game` that is belongs to the caller.
//!
//! The socket is the only thing a room is reached through, so creating a
//! room is a message on it like any other and there is no HTTP client
//! anywhere in the game.

use crate::ai::Intent;
use crate::level::Mission;
use crate::net::codec::Msg;
use crate::net::delta::apply_delta;
use crate::net::rooms::RoomCode;
use crate::net::transport::{Closed, ConnState, Transport};
use crate::net::wire::{IntentMsg, Lobby, RosterSeat, RoundOutcome, Snapshot, Welcome};

/// How many consecutive ticks a press is sent for
/// (docs/online-coop-prd.md §4.1). The server samples one intent per
/// tick and a press shorter than the gap between two packets would
/// otherwise arrive pressed and released inside the same sample, so a
/// tap is spread over two.
pub const FIRE_HOLD_TICKS: u8 = 2;

/// Who this client is to a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The name on the roster.
    pub nick: String,
    /// The reconnect key: the same token in a later `Join` reclaims the
    /// seat for the room's life. Minted once per player and kept (the
    /// page's `localStorage`, the app's keychain), never per session.
    pub device_token: String,
}

impl Identity {
    /// A player under `nick`, reconnecting with `device_token`.
    pub fn new(nick: impl Into<String>, device_token: impl Into<String>) -> Identity {
        Identity { nick: nick.into(), device_token: device_token.into() }
    }
}

/// What a host asks for when it opens a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomSetup {
    /// A shipped map's name (`map::SHIPPED_MAPS`).
    pub map: String,
    /// A builder map instead, carried whole; the room sends it on to
    /// every seat in the `Welcome`.
    pub map_toml: Option<String>,
    pub mission: Mission,
    /// Pin the round's seed (a replay, a test); absent, the room draws
    /// one.
    pub seed: Option<u64>,
}

impl Default for RoomSetup {
    fn default() -> RoomSetup {
        RoomSetup { map: "default".into(), map_toml: None, mission: Mission::Protect, seed: None }
    }
}

/// What a client is dialling a rooms server for: a room of its own on
/// these terms, or a seat in the room a code names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Host(RoomSetup),
    Join(RoomCode),
}

/// Open a socket to the rooms server `host` and greet it.
///
/// **The one place a client socket is opened.** The command line's
/// `--host`/`--join`, the lobby's `HOST` and `JOIN` buttons and the dev
/// server's `click` all come through here, so a button and a flag reach a
/// room by exactly the same path - `rooms::socket_url` for the URL, a
/// `NativeTransport` thread for the socket, this client for the lobby.
#[cfg(all(feature = "online", not(target_os = "emscripten")))]
pub fn connect(host: &crate::net::rooms::RoomsHost, identity: Identity, target: Target) -> RoomClient<Box<dyn Transport>> {
    let code = match &target {
        Target::Join(code) => Some(code),
        Target::Host(_) => None,
    };
    let url = crate::net::rooms::socket_url(host, code);
    eprintln!("[online] dialling {url}");
    let socket = Box::new(crate::net::native::NativeTransport::connect(&url)) as Box<dyn Transport>;
    match target {
        Target::Host(setup) => RoomClient::host(socket, identity, setup),
        Target::Join(code) => RoomClient::join(socket, identity, code.text),
    }
}

/// What the client is asking the room for, until it has asked.
#[derive(Clone, Debug)]
enum Greeting {
    Host(RoomSetup),
    Join(String),
}

/// How far along the client is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Dialling; nothing has been said yet.
    Connecting,
    /// The socket is open and the create or join has gone out; waiting
    /// for the room's answer.
    Greeting,
    /// In the room with a seat, before the round.
    Lobby,
    /// The round is running and snapshots are arriving.
    Playing,
    /// The socket is gone, with why.
    Closed(Closed),
}

/// What one `poll` saw.
#[derive(Clone, Debug, PartialEq)]
pub enum ClientEvent {
    /// The room exists (a host's answer to its own create); this is the
    /// code to share. A `Welcomed` follows.
    Created { code: String },
    /// The room's parameters and this client's seat. Build the replica
    /// from it with `net::apply::welcome`; its own snapshot is the
    /// baseline and is not repeated as a `Snapshot` event.
    Welcomed(Box<Welcome>),
    /// The roster changed: a seat joined, left, readied, dropped or came
    /// back.
    Roster { host: u8, seats: Vec<RosterSeat> },
    /// The host started the round; a `Welcomed` with the round's
    /// parameters follows.
    Started,
    /// The round is over and the room is back in its lobby, with how it
    /// went. The snapshots stop here; the phase is `Lobby` again, so a
    /// host can ask for the rematch on the same seat and the same code.
    Ended { outcome: RoundOutcome },
    /// A whole snapshot, deltas already applied onto the one before it.
    Snapshot(Box<Snapshot>),
    /// Somebody said something.
    Said { seat: u8, text: String },
    /// The room refused what was asked (a code that names no room, a
    /// full room, a kick of oneself). The socket stays open unless a
    /// `Closed` follows.
    Refused(String),
    /// The connection ended.
    Closed(Closed),
}

/// A seat in a room, over one socket.
pub struct RoomClient<T: Transport> {
    transport: T,
    identity: Identity,
    /// Held until the socket opens, then said once.
    greeting: Option<Greeting>,
    phase: Phase,
    code: Option<String>,
    seat: Option<u8>,
    host_seat: u8,
    seats: Vec<RosterSeat>,
    /// The last whole snapshot: what a delta is applied onto.
    baseline: Option<Snapshot>,
    /// The round has begun, so the next welcome is the round's.
    started: bool,
    intent_tick: u32,
    fire_held: u8,
    /// Reused by `poll` so a frame allocates nothing.
    scratch: Vec<Msg>,
}

impl<T: Transport> RoomClient<T> {
    /// Open a room on `transport`, which may still be connecting: the
    /// create goes out on the frame the socket opens.
    pub fn host(transport: T, identity: Identity, setup: RoomSetup) -> RoomClient<T> {
        RoomClient::new(transport, identity, Greeting::Host(setup))
    }

    /// Take a seat in the room `code` names. The code travels as given;
    /// `rooms::RoomCode::parse` is what canonicalises it and picks the
    /// URL the transport was dialled on.
    pub fn join(transport: T, identity: Identity, code: impl Into<String>) -> RoomClient<T> {
        RoomClient::new(transport, identity, Greeting::Join(code.into()))
    }

    fn new(transport: T, identity: Identity, greeting: Greeting) -> RoomClient<T> {
        RoomClient {
            transport,
            identity,
            greeting: Some(greeting),
            phase: Phase::Connecting,
            code: None,
            seat: None,
            host_seat: 0,
            seats: Vec::new(),
            baseline: None,
            started: false,
            intent_tick: 0,
            fire_held: 0,
            scratch: Vec::new(),
        }
    }

    /// Take the frame's turn: greet the room if the socket has just
    /// opened, then append everything that arrived to `out`, oldest
    /// first. Returns at once; `out` is appended to, not cleared.
    pub fn poll(&mut self, out: &mut Vec<ClientEvent>) {
        if matches!(self.phase, Phase::Closed(_)) {
            return;
        }
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        self.transport.drain(&mut scratch);
        if self.transport.is_open()
            && self.phase == Phase::Connecting
            && let Some(greeting) = self.greeting.take()
        {
            self.say_hello(&greeting);
            self.phase = Phase::Greeting;
        }
        for msg in scratch.drain(..) {
            self.take(msg, out);
        }
        self.scratch = scratch;
        if let ConnState::Closed(closed) = self.transport.state() {
            self.phase = Phase::Closed(closed.clone());
            out.push(ClientEvent::Closed(closed));
        }
    }

    /// This frame's input for the local seat, sent as one `IntentMsg`.
    ///
    /// The tick is the client's own count of the intents it has sent,
    /// and fire is held for `FIRE_HOLD_TICKS` so a tap survives the
    /// server's sampling. Call it once per frame while the round runs;
    /// a client with no seat yet or a closed socket sends nothing, so
    /// the count is the round's and not the lobby's.
    pub fn send_intent(&mut self, intent: &Intent) {
        if self.seat.is_none() || !self.transport.is_open() {
            return;
        }
        let mut msg = IntentMsg::new(self.intent_tick, intent);
        if msg.fire {
            self.fire_held = FIRE_HOLD_TICKS.saturating_sub(1);
        } else if self.fire_held > 0 {
            self.fire_held -= 1;
            msg.fire = true;
        }
        self.intent_tick = self.intent_tick.wrapping_add(1);
        self.transport.send_msg(&Msg::Intent(msg));
    }

    /// Say this seat is ready for the round.
    pub fn ready(&mut self) {
        self.say(Lobby::Ready);
    }

    /// Start the round (the host's to give).
    pub fn start(&mut self) {
        self.say(Lobby::Start);
    }

    /// Put a seat out of the room (the host's to give).
    pub fn kick(&mut self, seat: u8) {
        self.say(Lobby::Kick { seat });
    }

    /// Say something to the room; it comes back as `Said` to everyone.
    pub fn chat(&mut self, text: impl Into<String>) {
        self.say(Lobby::Chat { text: text.into() });
    }

    /// Give up the seat. The room answers by closing the socket, so the
    /// next `poll` reports `Closed`.
    pub fn leave(&mut self) {
        self.say(Lobby::Leave);
    }

    /// Drop the connection without a word (the window closed, the round
    /// abandoned).
    pub fn close(&mut self) {
        self.transport.close();
        self.phase = Phase::Closed(Closed::by_us());
    }

    /// How far along the client is.
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The room's code, once it has one: what a host shares and a
    /// joiner typed.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// The seat this client was given, once the room has said. It is the
    /// owner slot of its tank in every snapshot.
    pub fn seat(&self) -> Option<u8> {
        self.seat
    }

    /// Every seat in the room, by seat number.
    pub fn roster(&self) -> &[RosterSeat] {
        &self.seats
    }

    /// The seat that holds the room.
    pub fn host_seat(&self) -> u8 {
        self.host_seat
    }

    /// Whether this client holds the room (start and kick are its).
    pub fn is_host(&self) -> bool {
        self.seat == Some(self.host_seat)
    }

    /// The newest whole snapshot, the one a delta will be applied onto.
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.baseline.as_ref()
    }

    /// The socket underneath, for a caller that wants its state.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The create or join this client was made with.
    fn say_hello(&mut self, greeting: &Greeting) {
        let Identity { nick, device_token } = self.identity.clone();
        let msg = match greeting {
            Greeting::Host(setup) => Lobby::Create {
                nick,
                device_token,
                map: setup.map.clone(),
                map_toml: setup.map_toml.clone(),
                mission: setup.mission,
                seed: setup.seed,
            },
            Greeting::Join(code) => {
                self.code = Some(code.clone());
                Lobby::Join { nick, device_token, code: code.clone() }
            }
        };
        self.transport.send_msg(&Msg::Lobby(msg));
    }

    /// Send a lobby message, if there is still a socket to send it on.
    fn say(&mut self, msg: Lobby) {
        if self.transport.is_open() {
            self.transport.send_msg(&Msg::Lobby(msg));
        }
    }

    /// One message from the room.
    fn take(&mut self, msg: Msg, out: &mut Vec<ClientEvent>) {
        match msg {
            Msg::Lobby(Lobby::RoomCreated { code }) => {
                self.code = Some(code.clone());
                out.push(ClientEvent::Created { code });
            }
            Msg::Lobby(Lobby::Roster { host, seats }) => {
                self.host_seat = host;
                self.seats = seats.clone();
                out.push(ClientEvent::Roster { host, seats });
            }
            Msg::Lobby(Lobby::Started) => {
                self.started = true;
                out.push(ClientEvent::Started);
            }
            Msg::Lobby(Lobby::Ended { outcome }) => {
                // Back in the room: the next round is a fresh `Welcome`,
                // so the next welcome must not be read as a joiner's.
                self.started = false;
                if self.phase == Phase::Playing {
                    self.phase = Phase::Lobby;
                }
                out.push(ClientEvent::Ended { outcome });
            }
            Msg::Lobby(Lobby::Said { seat, text }) => out.push(ClientEvent::Said { seat, text }),
            Msg::Lobby(Lobby::Error { message }) => out.push(ClientEvent::Refused(message)),
            Msg::Welcome(welcome) => {
                self.seat = Some(welcome.seat);
                self.merge_roster(&welcome);
                // A round that is already running welcomes a joiner at
                // the tick it stands on, so the snapshot says as much as
                // `Started` does.
                self.phase = if self.started || welcome.snapshot.tick > 0 {
                    Phase::Playing
                } else {
                    Phase::Lobby
                };
                self.baseline = Some(welcome.snapshot.clone());
                out.push(ClientEvent::Welcomed(Box::new(welcome)));
            }
            Msg::Snapshot(snapshot) => {
                self.baseline = Some(snapshot.clone());
                out.push(ClientEvent::Snapshot(Box::new(snapshot)));
            }
            Msg::Delta(delta) => {
                // A delta is cut against the snapshot before it, so one
                // that does not follow this baseline is not this
                // client's: the room sends a full snapshot whenever it
                // has skipped one, and that resets the chain.
                let Some(baseline) = self.baseline.as_ref().filter(|b| b.tick < delta.tick) else {
                    return;
                };
                let next = apply_delta(baseline, &delta);
                self.baseline = Some(next.clone());
                out.push(ClientEvent::Snapshot(Box::new(next)));
            }
            // A room is told intents, never told them.
            Msg::Intent(_) => {}
            // The lobby's client half never comes back from a room.
            Msg::Lobby(_) => {}
        }
    }

    /// The welcome's roster into the lobby's, which is the one a caller
    /// reads. `Welcome::roster` carries a seat's name and chassis but
    /// not whether it is ready or connected, so those stand at "here,
    /// not ready" until the room's next `Roster`.
    fn merge_roster(&mut self, welcome: &Welcome) {
        for seat in &welcome.roster {
            match self.seats.iter_mut().find(|s| s.seat == seat.seat) {
                Some(known) => {
                    known.nick = seat.nick.clone();
                    known.chassis = seat.chassis;
                }
                None => self.seats.push(RosterSeat {
                    seat: seat.seat,
                    nick: seat.nick.clone(),
                    chassis: seat.chassis,
                    ready: false,
                    connected: true,
                }),
            }
        }
        self.seats.sort_by_key(|s| s.seat);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::codec;
    use crate::net::delta::delta;
    use crate::net::loopback::{self, LinkQuality};
    use crate::net::transport::Transport;
    use crate::net::wire::{RoundState, Seat, TankState};
    use crate::net::{MAX_SEATS, PROTOCOL_VERSION};
    use crate::tank::Dir;

    /// The room's end of a loopback link, answering by hand.
    struct Room {
        link: loopback::Loopback,
        heard: Vec<Msg>,
    }

    impl Room {
        fn say(&mut self, msg: Msg) {
            self.link.send(&codec::encode(&msg));
        }

        fn lobby(&mut self, msg: Lobby) {
            self.say(Msg::Lobby(msg));
        }

        /// Everything the client has said since the last look.
        fn listen(&mut self) -> Vec<Msg> {
            let mut heard = Vec::new();
            self.link.drain(&mut heard);
            self.heard.extend(heard.iter().cloned());
            heard
        }
    }

    fn room_and_client() -> (Room, RoomClient<loopback::Loopback>) {
        let (client_end, room_end) = loopback::pair(LinkQuality::PERFECT, 1);
        let room = Room { link: room_end, heard: Vec::new() };
        let client = RoomClient::host(
            client_end,
            Identity::new("host", "tok-host"),
            RoomSetup { seed: Some(0xB0B5), ..RoomSetup::default() },
        );
        (room, client)
    }

    fn snapshot(tick: u32) -> Snapshot {
        Snapshot {
            tick,
            tanks: vec![
                TankState { id: 0, x: 400 + tick as i16, y: 800, hp: 100, ..Default::default() },
                TankState { id: 1, x: 900, y: 800, hp: 100, ..Default::default() },
            ],
            round: RoundState { wave: 1, alive: 2, ..Default::default() },
            ..Default::default()
        }
    }

    fn welcome(seat: u8, tick: u32) -> Welcome {
        Welcome {
            protocol: PROTOCOL_VERSION,
            sim_version: "0.1.0".into(),
            seat,
            roster: vec![
                Seat { seat: 0, nick: "host".into(), chassis: 3 },
                Seat { seat: 1, nick: "second".into(), chassis: 5 },
            ],
            map_toml: "size = [34, 17]\n".into(),
            seed: 0xB0B5,
            tuning_json: "{}".into(),
            overrides: Default::default(),
            enemy_count: None,
            oil_cells: Vec::new(),
            dead_cells: Vec::new(),
            snapshot: snapshot(tick),
        }
    }

    /// One frame: the client polls, the room reads what it said.
    fn frame(room: &mut Room, client: &mut RoomClient<loopback::Loopback>) -> Vec<ClientEvent> {
        let mut out = Vec::new();
        client.poll(&mut out);
        room.listen();
        out
    }

    #[test]
    fn hosting_walks_connect_to_greeting_to_lobby_to_playing() {
        let (mut room, mut client) = room_and_client();
        assert_eq!(client.phase(), &Phase::Connecting);

        // The first poll finds the socket open and says hello.
        let events = frame(&mut room, &mut client);
        assert!(events.is_empty());
        assert_eq!(client.phase(), &Phase::Greeting);
        assert_eq!(
            room.heard,
            vec![Msg::Lobby(Lobby::Create {
                nick: "host".into(),
                device_token: "tok-host".into(),
                map: "default".into(),
                map_toml: None,
                mission: Mission::Protect,
                seed: Some(0xB0B5),
            })],
            "the create carries the identity and the setup"
        );

        // A second poll must not greet twice.
        frame(&mut room, &mut client);
        assert_eq!(room.heard.len(), 1);

        // The room answers with the code and a lobby welcome.
        room.lobby(Lobby::RoomCreated { code: "AK7QX".into() });
        room.say(Msg::Welcome(welcome(0, 0)));
        let events = frame(&mut room, &mut client);
        assert_eq!(events[0], ClientEvent::Created { code: "AK7QX".into() });
        assert!(matches!(&events[1], ClientEvent::Welcomed(w) if w.seat == 0));
        assert_eq!(events.len(), 2);
        assert_eq!(client.phase(), &Phase::Lobby);
        assert_eq!(client.code(), Some("AK7QX"));
        assert_eq!(client.seat(), Some(0));
        assert!(client.is_host());
        assert_eq!(client.snapshot().map(|s| s.tick), Some(0), "the welcome's snapshot is the baseline");
        assert_eq!(
            client.roster().iter().map(|s| (s.seat, s.nick.as_str(), s.chassis)).collect::<Vec<_>>(),
            [(0, "host", 3), (1, "second", 5)],
            "the welcome's roster is what the lobby shows until the room sends one"
        );

        // Ready and start are lobby messages.
        client.ready();
        client.start();
        frame(&mut room, &mut client);
        assert_eq!(room.heard[1..], [Msg::Lobby(Lobby::Ready), Msg::Lobby(Lobby::Start)]);

        // The round begins: Started, then the round's welcome.
        room.lobby(Lobby::Started);
        room.say(Msg::Welcome(welcome(0, 0)));
        let events = frame(&mut room, &mut client);
        assert_eq!(events[0], ClientEvent::Started);
        assert!(matches!(events[1], ClientEvent::Welcomed(_)));
        assert_eq!(client.phase(), &Phase::Playing);
    }

    #[test]
    fn joining_a_running_round_lands_in_playing_without_a_started() {
        let (client_end, room_end) = loopback::pair(LinkQuality::PERFECT, 2);
        let mut room = Room { link: room_end, heard: Vec::new() };
        let mut client =
            RoomClient::join(client_end, Identity::new("second", "tok-second"), "ak7qx");
        frame(&mut room, &mut client);
        assert_eq!(
            room.heard,
            vec![Msg::Lobby(Lobby::Join {
                nick: "second".into(),
                device_token: "tok-second".into(),
                code: "ak7qx".into(),
            })],
            "the code travels as the caller spelled it; the room canonicalises"
        );
        assert_eq!(client.code(), Some("ak7qx"));

        room.say(Msg::Welcome(welcome(1, 1_200)));
        let events = frame(&mut room, &mut client);
        assert!(matches!(&events[0], ClientEvent::Welcomed(w) if w.seat == 1));
        assert_eq!(client.phase(), &Phase::Playing, "a welcome cut mid-round is the round");
        assert_eq!(client.seat(), Some(1));
        assert!(!client.is_host());
    }

    #[test]
    fn deltas_are_applied_onto_the_baseline_so_a_caller_sees_whole_snapshots() {
        let (mut room, mut client) = room_and_client();
        frame(&mut room, &mut client);
        room.lobby(Lobby::RoomCreated { code: "AK7QX".into() });
        room.lobby(Lobby::Started);
        room.say(Msg::Welcome(welcome(0, 0)));
        frame(&mut room, &mut client);

        let (mut prev, mut sent) = (snapshot(0), Vec::new());
        for tick in [3u32, 6, 9] {
            let next = snapshot(tick);
            room.say(Msg::Delta(delta(&prev, &next)));
            prev = next.clone();
            sent.push(next);
        }
        let events = frame(&mut room, &mut client);
        let seen: Vec<&Snapshot> = events
            .iter()
            .map(|e| match e {
                ClientEvent::Snapshot(s) => s.as_ref(),
                other => panic!("not a snapshot: {other:?}"),
            })
            .collect();
        assert_eq!(seen.iter().map(|s| s.tick).collect::<Vec<_>>(), [3, 6, 9]);
        assert_eq!(seen[2].tanks, sent[2].tanks, "the chain rebuilt the room's own snapshot");
        assert_eq!(client.snapshot().unwrap().tick, 9);

        // A full snapshot resets the baseline; a delta that does not
        // follow it is not this client's and is left alone.
        room.say(Msg::Snapshot(snapshot(30)));
        room.say(Msg::Delta(delta(&snapshot(9), &snapshot(12))));
        let events = frame(&mut room, &mut client);
        assert_eq!(events.len(), 1, "only the full snapshot came through: {events:?}");
        assert_eq!(client.snapshot().unwrap().tick, 30);
    }

    #[test]
    fn a_refusal_is_an_event_and_the_socket_lives_on() {
        let (client_end, room_end) = loopback::pair(LinkQuality::PERFECT, 3);
        let mut room = Room { link: room_end, heard: Vec::new() };
        let mut client = RoomClient::join(client_end, Identity::new("x", "tok-x"), "AKKKK");
        frame(&mut room, &mut client);
        room.lobby(Lobby::Error { message: "no room AKKKK".into() });
        let events = frame(&mut room, &mut client);
        assert_eq!(events, [ClientEvent::Refused("no room AKKKK".into())]);
        assert_eq!(client.phase(), &Phase::Greeting, "still connected, still nobody's seat");
        assert_eq!(client.seat(), None);
    }

    #[test]
    fn a_closed_socket_ends_the_client_once_and_after_the_last_words() {
        let (mut room, mut client) = room_and_client();
        frame(&mut room, &mut client);
        room.lobby(Lobby::Error { message: "the room is draining".into() });
        room.link.close();
        let events = frame(&mut room, &mut client);
        assert_eq!(events[0], ClientEvent::Refused("the room is draining".into()));
        assert!(matches!(events[1], ClientEvent::Closed(_)), "the reason arrives before the end");
        assert_eq!(events.len(), 2);
        assert!(matches!(client.phase(), Phase::Closed(_)));
        // Nothing more is reported, and nothing more is said.
        let events = frame(&mut room, &mut client);
        assert!(events.is_empty());
        client.ready();
        client.send_intent(&Intent::default());
        assert!(room.listen().is_empty());
    }

    #[test]
    fn the_roster_follows_the_rooms_own_and_names_the_host() {
        let (mut room, mut client) = room_and_client();
        frame(&mut room, &mut client);
        room.say(Msg::Welcome(welcome(1, 0)));
        room.lobby(Lobby::Roster {
            host: 1,
            seats: vec![
                RosterSeat { seat: 0, nick: "host".into(), chassis: 3, ready: true, connected: false },
                RosterSeat { seat: 1, nick: "second".into(), chassis: 5, ready: true, connected: true },
            ],
        });
        let events = frame(&mut room, &mut client);
        assert!(matches!(events[1], ClientEvent::Roster { host: 1, .. }));
        assert_eq!(client.host_seat(), 1);
        assert!(client.is_host(), "the host seat moved to this client");
        assert!(client.roster()[0].ready && !client.roster()[0].connected);
    }

    #[test]
    fn an_intent_goes_out_every_tick_and_a_tap_is_held_for_two() {
        let (mut room, mut client) = room_and_client();
        frame(&mut room, &mut client);
        // Nothing is sent before the room has given this client a seat.
        client.send_intent(&Intent::default());
        assert!(room.listen().is_empty());
        room.say(Msg::Welcome(welcome(0, 0)));
        frame(&mut room, &mut client);
        let mut taps = vec![false; 6];
        taps[1] = true;
        let mut fired = Vec::new();
        for tap in taps {
            client.send_intent(&Intent { move_dir: Some(Dir::Up), fire: tap, ..Intent::default() });
            for msg in room.listen() {
                match msg {
                    Msg::Intent(i) => fired.push((i.tick, i.move_dir, i.fire)),
                    other => panic!("not an intent: {other:?}"),
                }
            }
        }
        assert_eq!(
            fired,
            [(0, 1, false), (1, 1, true), (2, 1, true), (3, 1, false), (4, 1, false), (5, 1, false)],
            "one intent per tick, the tap held for {FIRE_HOLD_TICKS}"
        );
    }

    #[test]
    fn a_message_out_of_the_lobbys_client_half_is_ignored() {
        let (mut room, mut client) = room_and_client();
        frame(&mut room, &mut client);
        // Things a room never says to a client, and a delta with no
        // baseline: none of them is an event, none of them is fatal.
        room.lobby(Lobby::Ready);
        room.say(Msg::Intent(IntentMsg::default()));
        room.say(Msg::Delta(delta(&snapshot(0), &snapshot(3))));
        let events = frame(&mut room, &mut client);
        assert!(events.is_empty(), "{events:?}");
        assert!(client.snapshot().is_none());
        assert_eq!(client.phase(), &Phase::Greeting);
    }

    #[test]
    fn a_welcome_tells_the_caller_the_seat_and_every_snapshot_keys_on_it() {
        let (mut room, mut client) = room_and_client();
        frame(&mut room, &mut client);
        room.say(Msg::Welcome(welcome(1, 0)));
        let events = frame(&mut room, &mut client);
        let ClientEvent::Welcomed(w) = &events[0] else { panic!("no welcome: {events:?}") };
        assert_eq!(w.seed, 0xB0B5);
        assert_eq!(w.snapshot.acked, [0; MAX_SEATS]);
        let seat = client.seat().expect("a seat");
        assert!(
            w.snapshot.tanks.iter().any(|t| t.id == seat as u16),
            "the seat is the owner slot of a tank in the snapshot"
        );
    }
}
