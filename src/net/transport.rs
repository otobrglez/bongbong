//! The one socket a client holds, behind one trait
//! (docs/online-coop-prd.md §4.6). Everything a client says to a room -
//! creating one included - is a message on it, so the game needs no HTTP
//! client and no second connection.
//!
//! The contract is shaped by the frame loop that calls it: `send` queues
//! bytes and returns, `drain` moves everything that has arrived since the
//! last call into a `Vec<Msg>` and returns immediately when nothing has.
//! Neither ever blocks, so a transport can sit in `Game::update`'s frame
//! the way `tuning::apply_pending` and the dev server's queue do: the
//! socket's own thread (native) or the browser's event loop (web) does
//! the waiting, and the frame boundary is the only place their state
//! crosses into the game.
//!
//! Three implementations share it: `native` (a `tungstenite` socket on
//! one std thread, desktop, iOS and Android), `web` (emscripten's
//! WebSocket API through FFI) and `loopback` (an in-process pair with
//! dialled delay, jitter and loss for the offline rig). `client` is the
//! lobby state machine over any of them.

use crate::net::codec::{self, Msg};

/// Where a connection stands. A transport reports what its last `drain`
/// saw: the frame drains, then reads the state, so both describe the
/// same moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnState {
    /// Dialling: the socket is being opened and nothing can arrive yet.
    /// Bytes handed to `send` now are queued for the moment it opens.
    Connecting,
    /// Open: messages travel both ways.
    Open,
    /// Gone for good, with why. A transport never re-dials by itself;
    /// reconnecting is opening a new one (the device token in
    /// `Lobby::Join` is what reclaims the seat).
    Closed(Closed),
}

/// Why a connection ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed {
    /// What to show a player: a close frame's reason, a handshake
    /// failure, a read error.
    pub reason: String,
    /// This end asked for it (`Transport::close`), rather than the room
    /// or the network.
    pub requested: bool,
}

impl Closed {
    /// The connection failed or the far end hung up.
    pub fn fault(reason: impl Into<String>) -> Closed {
        Closed { reason: reason.into(), requested: false }
    }

    /// This end closed the socket.
    pub fn by_us() -> Closed {
        Closed { reason: "closed by this end".into(), requested: true }
    }
}

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

/// A socket that was never opened, carrying why.
///
/// There is exactly one moment a transport can fail before it exists,
/// and it is the browser's: `new WebSocket(url)` throws on a URL it will
/// not dial - a plain `ws://` from a page served over TLS, a host it
/// cannot parse - so `net::web`'s `connect` answers from the call rather
/// than from a callback. The native thread has no such moment; it dials
/// in the background and reports a failure as a close like any other.
///
/// So rather than hand every caller a `Result` for one platform's sake,
/// the failure is a transport: `Closed` from the first frame, with the
/// reason where a hang-up's would be, so the lobby's "the room is gone"
/// face says what happened along the path everything else takes.
pub struct Failed {
    closed: Closed,
}

impl Failed {
    /// A connection that never happened, because `reason`.
    pub fn new(reason: impl Into<String>) -> Failed {
        Failed { closed: Closed::fault(reason) }
    }
}

impl Transport for Failed {
    fn state(&self) -> ConnState {
        ConnState::Closed(self.closed.clone())
    }

    /// Nothing is sent on a socket that does not exist.
    fn send(&mut self, _bytes: &[u8]) {}

    /// Nothing ever arrives.
    fn drain(&mut self, _out: &mut Vec<Msg>) {}

    /// Already closed, and by the browser rather than by this end: the
    /// reason stays the one worth reading.
    fn close(&mut self) {}
}

/// A client's socket to a room.
///
/// Implementations deliver whole messages in the order they were sent
/// (every one is a WebSocket frame) and never block the caller. A
/// message that does not decode is dropped rather than fatal: §4.3's
/// rule is that a new `Lobby` variant needs no `PROTOCOL_VERSION` bump,
/// so a room one build ahead may say something this build has no name
/// for, and the round must not end over it.
pub trait Transport {
    /// Where the connection stands, as of the last `drain`.
    fn state(&self) -> ConnState;

    /// Queue `bytes` as one message. Returns at once; a transport that
    /// is still connecting sends them when it opens, and one that is
    /// closed drops them.
    fn send(&mut self, bytes: &[u8]);

    /// Move everything that has arrived since the last call into `out`,
    /// oldest first, and return. `out` is appended to, not cleared.
    fn drain(&mut self, out: &mut Vec<Msg>);

    /// Close the connection. Further `send`s are dropped and `state`
    /// settles on `Closed` with `requested` set.
    fn close(&mut self);

    /// `send` of an encoded message.
    fn send_msg(&mut self, msg: &Msg) {
        self.send(&codec::encode(msg));
    }

    /// Whether messages travel right now.
    fn is_open(&self) -> bool {
        self.state() == ConnState::Open
    }

    /// Why the connection ended, if it has.
    fn closed(&self) -> Option<Closed> {
        match self.state() {
            ConnState::Closed(c) => Some(c),
            _ => None,
        }
    }
}

/// A boxed transport is a transport, so one caller can hold any of them
/// behind `Box<dyn Transport>`: the window's online round is the same
/// type whether it plays over a socket or over the rig's in-process link.
impl<T: Transport + ?Sized> Transport for Box<T> {
    fn state(&self) -> ConnState {
        (**self).state()
    }

    fn send(&mut self, bytes: &[u8]) {
        (**self).send(bytes);
    }

    fn drain(&mut self, out: &mut Vec<Msg>) {
        (**self).drain(out);
    }

    fn close(&mut self) {
        (**self).close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::wire::IntentMsg;

    /// A transport that answers every message with itself.
    struct Echo {
        state: ConnState,
        queue: Vec<Vec<u8>>,
    }

    impl Echo {
        fn open() -> Echo {
            Echo { state: ConnState::Open, queue: Vec::new() }
        }
    }

    impl Transport for Echo {
        fn state(&self) -> ConnState {
            self.state.clone()
        }

        fn send(&mut self, bytes: &[u8]) {
            if self.state == ConnState::Open {
                self.queue.push(bytes.to_vec());
            }
        }

        fn drain(&mut self, out: &mut Vec<Msg>) {
            out.extend(self.queue.drain(..).filter_map(|b| codec::decode(&b).ok()));
        }

        fn close(&mut self) {
            self.state = ConnState::Closed(Closed::by_us());
        }
    }

    #[test]
    fn send_msg_encodes_and_drain_decodes() {
        let mut t = Echo::open();
        let msg = Msg::Intent(IntentMsg { tick: 4, move_dir: 2, face: 2, fire: true });
        t.send_msg(&msg);
        let mut out = vec![Msg::Intent(IntentMsg::default())];
        t.drain(&mut out);
        assert_eq!(out.len(), 2, "drain appends");
        assert_eq!(out[1], msg);
        let mut again = Vec::new();
        t.drain(&mut again);
        assert!(again.is_empty(), "an empty drain returns nothing, not the last batch again");
    }

    /// A browser that would not open the socket at all still has to
    /// reach the screen, and it does it as an ordinary closed
    /// connection.
    #[test]
    fn a_socket_that_was_never_opened_is_a_closed_one_with_the_reason() {
        let mut t = Failed::new("the browser refused a socket to ws://host/ws");
        assert!(!t.is_open());
        let closed = t.closed().expect("a reason");
        assert!(closed.reason.contains("ws://host/ws"), "{closed}");
        assert!(!closed.requested, "this end never asked for it");
        // Nothing travels, and hanging up keeps the reason worth reading.
        t.send_msg(&Msg::Intent(IntentMsg::default()));
        let mut out = Vec::new();
        t.drain(&mut out);
        assert!(out.is_empty());
        t.close();
        assert_eq!(t.closed(), Some(closed));
    }

    #[test]
    fn closing_stops_the_socket_and_says_who_asked() {
        let mut t = Echo::open();
        assert!(t.is_open());
        assert_eq!(t.closed(), None);
        t.close();
        assert!(!t.is_open());
        let closed = t.closed().expect("a reason");
        assert!(closed.requested);
        assert!(!closed.to_string().is_empty());
        t.send_msg(&Msg::Intent(IntentMsg::default()));
        let mut out = Vec::new();
        t.drain(&mut out);
        assert!(out.is_empty(), "a closed transport takes nothing");
        assert!(!Closed::fault("the room is gone").requested);
    }
}
