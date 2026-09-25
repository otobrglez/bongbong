//! The native socket (docs/online-coop-prd.md §4.6): blocking
//! `tungstenite` with `rustls` and webpki's roots on one std thread that
//! owns it, talking to the frame through two mpsc channels - the dev
//! server's pattern, and the reason the client needs no async runtime.
//! macOS, Linux, Windows, iOS and Android all run this; only emscripten
//! has its own path.
//!
//! The thread's loop is flush-then-read: it drains the outgoing queue,
//! writes it, then reads with a short timeout, so an intent leaves
//! within `READ_TIMEOUT` of the frame that made it and nothing waits on
//! the socket but the thread. A read timeout arrives as `WouldBlock`,
//! which is not an error but the quiet between messages; tungstenite
//! keeps a half-read frame in its own buffer across one, and rustls its
//! half-read record, so the loop can retry forever.
//!
//! What crosses the channel is bytes, not messages: `drain` decodes on
//! the frame's thread through `net::codec`, so a message the socket
//! cannot make sense of costs the round nothing.

use std::io::ErrorKind;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread;
use std::time::Duration;

use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as WsError, Message};

use crate::net::codec::{self, Msg};
use crate::net::transport::{Closed, ConnState, Transport};

/// How long the socket thread waits on a read before it looks at the
/// outgoing queue again. Short enough that an intent made this frame
/// leaves on this frame, long enough that an idle connection is not a
/// spin loop.
const READ_TIMEOUT: Duration = Duration::from_millis(5);

/// What the frame asks the socket thread to do.
enum Cmd {
    Send(Vec<u8>),
    Close,
}

/// What the socket thread tells the frame.
enum Wire {
    Open,
    Bytes(Vec<u8>),
    Closed(Closed),
}

/// A WebSocket to a room, run by its own thread.
///
/// Connecting is part of the thread's work, so `connect` returns at once
/// and the state is `Connecting` until the handshake lands. Dropping the
/// transport closes the socket and ends the thread.
pub struct NativeTransport {
    cmds: Sender<Cmd>,
    wire: Receiver<Wire>,
    state: ConnState,
}

impl NativeTransport {
    /// Dial `url` (`ws://` or `wss://`, from `rooms::socket_url`).
    ///
    /// Returns before the handshake: the thread does the DNS lookup, the
    /// TCP connect, the TLS handshake and the upgrade, and the frame
    /// sees `Connecting` until then. Bytes handed to `send` meanwhile
    /// are queued and go out in order the moment the socket opens, so a
    /// caller can dial and greet in the same breath.
    pub fn connect(url: &str) -> NativeTransport {
        let (cmds, rx) = channel();
        let (tx, wire) = channel();
        let url = url.to_string();
        thread::Builder::new()
            .name("bongbong-room-socket".into())
            .spawn(move || run(url, rx, tx))
            .expect("the room socket thread starts");
        NativeTransport { cmds, wire, state: ConnState::Connecting }
    }

    /// Take everything the socket thread has said, updating the state
    /// and handing back the bytes that arrived.
    fn pump(&mut self, mut bytes: impl FnMut(Vec<u8>)) {
        loop {
            match self.wire.try_recv() {
                Ok(Wire::Open) => self.state = ConnState::Open,
                Ok(Wire::Bytes(b)) => bytes(b),
                Ok(Wire::Closed(c)) => self.state = ConnState::Closed(c),
                Err(TryRecvError::Empty) => return,
                // The thread is gone without a word: it panicked, or the
                // process is coming down. Either way the socket is over.
                Err(TryRecvError::Disconnected) => {
                    if !matches!(self.state, ConnState::Closed(_)) {
                        self.state = ConnState::Closed(Closed::fault("the socket thread stopped"));
                    }
                    return;
                }
            }
        }
    }
}

impl Transport for NativeTransport {
    fn state(&self) -> ConnState {
        self.state.clone()
    }

    fn send(&mut self, bytes: &[u8]) {
        if matches!(self.state, ConnState::Closed(_)) {
            return;
        }
        let _ = self.cmds.send(Cmd::Send(bytes.to_vec()));
    }

    fn drain(&mut self, out: &mut Vec<Msg>) {
        let mut arrived = Vec::new();
        self.pump(|b| arrived.push(b));
        out.extend(arrived.iter().filter_map(|b| codec::decode(b).ok()));
    }

    fn close(&mut self) {
        let _ = self.cmds.send(Cmd::Close);
        self.state = ConnState::Closed(Closed::by_us());
    }
}

impl Drop for NativeTransport {
    fn drop(&mut self) {
        let _ = self.cmds.send(Cmd::Close);
    }
}

/// The socket thread: connect, then flush and read until either end
/// hangs up. Every exit tells the frame why.
fn run(url: String, cmds: Receiver<Cmd>, wire: Sender<Wire>) {
    let mut socket = match dial(&url) {
        Ok(socket) => socket,
        Err(reason) => {
            let _ = wire.send(Wire::Closed(Closed::fault(reason)));
            return;
        }
    };
    if wire.send(Wire::Open).is_err() {
        return;
    }
    let closed = loop {
        // Everything the frame queued since the last pass, in order.
        // A `Close` is obeyed at once: what is behind it never mattered.
        let mut closing = false;
        let mut failed: Option<WsError> = None;
        loop {
            match cmds.try_recv() {
                Ok(Cmd::Send(bytes)) => {
                    if let Err(e) = socket.write(Message::binary(bytes)) {
                        failed = Some(e);
                        break;
                    }
                }
                // The frame asked to go, or it dropped the transport
                // and there is nobody left to talk for.
                Ok(Cmd::Close) | Err(TryRecvError::Disconnected) => {
                    closing = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        if closing {
            let _ = socket.close(None);
            let _ = socket.flush();
            break Closed::by_us();
        }
        if let Some(e) = failed.or_else(|| socket.flush().err())
            && !would_block(&e)
        {
            break Closed::fault(format!("writing to the room failed: {e}"));
        }
        match socket.read() {
            Ok(Message::Binary(bytes)) => {
                if wire.send(Wire::Bytes(bytes.to_vec())).is_err() {
                    break Closed::by_us();
                }
            }
            // The room's last word: a refusal is queued and the socket
            // closed in the same breath, so the bytes above have already
            // gone to the frame and this carries only the why.
            Ok(Message::Close(frame)) => {
                let _ = socket.flush();
                break match frame {
                    Some(f) if !f.reason.is_empty() => Closed::fault(f.reason.to_string()),
                    _ => Closed::fault("the room closed the connection"),
                };
            }
            // A room speaks binary; a text frame is something else
            // talking, and the ping and pong tungstenite answers itself
            // are not the frame's business.
            Ok(_) => {}
            Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => {
                break Closed::fault("the room closed the connection");
            }
            Err(e) if would_block(&e) => {}
            Err(e) => break Closed::fault(format!("the connection to the room failed: {e}")),
        }
    };
    let _ = wire.send(Wire::Closed(closed));
}

/// Open the socket and put a read timeout on it, so the thread's loop
/// comes back to the outgoing queue even on a silent connection.
fn dial(url: &str) -> Result<tungstenite::WebSocket<MaybeTlsStream<std::net::TcpStream>>, String> {
    // rustls comes in through tungstenite with no default features, so
    // nothing has chosen a crypto provider yet; `ring` is the one this
    // crate builds with. Installing it twice is not an error worth
    // reporting - the second caller simply finds the first's.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (socket, _response) =
        tungstenite::connect(url).map_err(|e| format!("cannot reach {url}: {e}"))?;
    let tcp = match socket.get_ref() {
        MaybeTlsStream::Plain(tcp) => tcp,
        MaybeTlsStream::Rustls(tls) => &tls.sock,
        // `MaybeTlsStream` grows a variant per TLS backend; this build
        // links rustls and nothing else.
        _ => return Err("the socket is on an unexpected TLS backend".into()),
    };
    tcp.set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|e| format!("cannot set the socket's read timeout: {e}"))?;
    Ok(socket)
}

/// Whether an error is a read that found nothing within the timeout -
/// the quiet between messages, not a fault. Which of the two kinds a
/// timed-out socket reports is the platform's business.
fn would_block(e: &WsError) -> bool {
    matches!(e, WsError::Io(io) if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_connection_closes_with_a_reason_instead_of_hanging() {
        // Port 1 on loopback takes no connections.
        let mut t = NativeTransport::connect("ws://127.0.0.1:1/ws");
        let mut out = Vec::new();
        for _ in 0..600 {
            t.drain(&mut out);
            if let ConnState::Closed(c) = t.state() {
                assert!(!c.requested, "the network refused it, not us");
                assert!(c.reason.contains("127.0.0.1:1"), "the reason names the address: {}", c.reason);
                assert!(out.is_empty());
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the transport never settled: {:?}", t.state());
    }

    #[test]
    fn a_url_that_is_not_a_socket_is_refused_the_same_way() {
        let mut t = NativeTransport::connect("http://127.0.0.1:1/ws");
        let mut out = Vec::new();
        for _ in 0..200 {
            t.drain(&mut out);
            if let ConnState::Closed(c) = t.state() {
                assert!(!c.reason.is_empty());
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the transport never settled");
    }

    #[test]
    fn closing_before_the_handshake_lands_is_not_a_panic() {
        let mut t = NativeTransport::connect("ws://127.0.0.1:1/ws");
        t.close();
        assert_eq!(t.state(), ConnState::Closed(Closed::by_us()));
        t.send(b"dropped");
        let mut out = Vec::new();
        t.drain(&mut out);
        assert!(out.is_empty());
    }
}
