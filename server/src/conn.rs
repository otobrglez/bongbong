//! One WebSocket connection (docs/online-coop-prd.md §4.7): a reader that
//! decodes every frame and routes it - an intent into the seat's mailbox,
//! a lobby message to the room's command channel - and a writer draining
//! a bounded `Outbox`. The room never awaits a client: it offers a
//! snapshot to the outbox and moves on, so a slow client skips snapshots
//! and never queues them.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use bongbong::map::{MapFile, open_map};
use bongbong::net::codec::{self, Msg};
use bongbong::net::wire::Lobby;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::hub::{Hub, RoomHandle};
use crate::mailbox::Mailbox;
use crate::room::{Command, ConnLink, RoomParams};

/// Messages the writer holds for a client before the room starts
/// skipping its snapshots: at 20 Hz, under a second of lag.
pub const OUTBOX_DEPTH: usize = 16;

/// How long the writer gets to flush what is queued and send the
/// WebSocket close frame once the reader has stopped. A refusal is
/// queued and the connection closed in the same breath (`remove_seat`
/// says why the seat went), so cutting the socket here would reset it
/// before the client ever read the reason.
const FLUSH_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// What became of a message offered to an outbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Sent,
    /// The queue was full; the message was dropped.
    Skipped,
    /// The connection is closed or was told to close.
    Gone,
}

/// The room's handle on a connection's writer: a bounded queue of encoded
/// messages and an out-of-band close signal, so a client whose queue is
/// full can still be told to go.
#[derive(Clone)]
pub struct Outbox {
    tx: mpsc::Sender<Bytes>,
    close: Arc<Notify>,
}

impl Outbox {
    /// An outbox and the writer's ends of it: the queue and the close
    /// signal.
    pub fn pair() -> (Outbox, mpsc::Receiver<Bytes>, Arc<Notify>) {
        let (tx, rx) = mpsc::channel(OUTBOX_DEPTH);
        let close = Arc::new(Notify::new());
        (Outbox { tx, close: close.clone() }, rx, close)
    }

    /// Offer a snapshot: queued if there is room, dropped otherwise.
    pub fn offer(&self, bytes: Bytes) -> Delivery {
        match self.tx.try_send(bytes) {
            Ok(()) => Delivery::Sent,
            Err(mpsc::error::TrySendError::Full(_)) => Delivery::Skipped,
            Err(mpsc::error::TrySendError::Closed(_)) => Delivery::Gone,
        }
    }

    /// Send a message that must not be lost (a welcome, a lobby reply).
    /// A client too far behind to take it is closed instead of queued
    /// further.
    pub fn send(&self, bytes: Bytes) -> Delivery {
        match self.offer(bytes) {
            Delivery::Skipped => {
                self.close();
                Delivery::Gone
            }
            other => other,
        }
    }

    /// Encode and `send` a lobby reply.
    pub fn lobby(&self, msg: Lobby) -> Delivery {
        self.send(Bytes::from(codec::encode(&Msg::Lobby(msg))))
    }

    /// Tell the connection task to close the socket.
    pub fn close(&self) {
        self.close.notify_one();
    }

    /// Whether the writer is still there.
    pub fn is_open(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// The connection's place in a room once it has joined one.
struct Attached {
    room: RoomHandle,
    mailbox: Arc<Mailbox>,
}

/// Run one connection to its end.
pub async fn run(socket: WebSocket, hub: Arc<Hub>) {
    let conn_id = hub.next_conn_id();
    hub.metrics.connections_total.fetch_add(1, Ordering::Relaxed);
    debug!(conn = conn_id, "connected");
    let (mut sink, mut stream) = socket.split();
    let (outbox, mut rx, close) = Outbox::pair();
    let writer = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if sink.send(Message::Binary(bytes)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let mut attached: Option<Attached> = None;
    loop {
        tokio::select! {
            frame = stream.next() => match frame {
                Some(Ok(Message::Binary(bytes))) => {
                    handle(&bytes, conn_id, &hub, &outbox, &mut attached).await;
                }
                Some(Ok(Message::Text(text))) => {
                    // A bare JSON lobby message, for a hand-driven client
                    // (`websocat`): the kind byte is implied.
                    let mut bytes = vec![codec::kind::LOBBY];
                    bytes.extend_from_slice(text.as_bytes());
                    handle(&bytes, conn_id, &hub, &outbox, &mut attached).await;
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
            },
            _ = close.notified() => break,
            _ = hub.shut_down() => break,
        }
    }
    if let Some(a) = &attached {
        let _ = a.room.commands.send(Command::Disconnected { conn_id }).await;
    }
    // Dropping every sender ends the writer's loop once it has drained
    // what is queued, and it closes the socket properly on its way out.
    drop(outbox);
    let aborter = writer.abort_handle();
    if tokio::time::timeout(FLUSH_GRACE, writer).await.is_err() {
        aborter.abort();
    }
    debug!(conn = conn_id, "closed");
}

async fn handle(bytes: &[u8], conn_id: u64, hub: &Arc<Hub>, outbox: &Outbox, attached: &mut Option<Attached>) {
    let msg = match codec::decode(bytes) {
        Ok(msg) => msg,
        Err(e) => {
            outbox.lobby(Lobby::Error { message: e.to_string() });
            return;
        }
    };
    match msg {
        Msg::Intent(intent) => {
            if let Some(a) = attached {
                a.mailbox.post(intent, Instant::now());
            }
        }
        Msg::Lobby(Lobby::Create { nick, device_token, map, map_toml, mission, seed }) => {
            if attached.is_some() {
                outbox.lobby(Lobby::Error { message: "already in a room".into() });
                return;
            }
            let map = match map_toml {
                Some(toml) => MapFile::from_toml_str(&toml),
                None => open_map(&map),
            };
            let map = match map {
                Ok(map) => map,
                Err(e) => {
                    outbox.lobby(Lobby::Error { message: format!("bad map: {e}") });
                    return;
                }
            };
            let params = RoomParams { map, mission, seed };
            let room = match hub.create_room(params) {
                Ok(room) => room,
                Err(message) => {
                    warn!(conn = conn_id, %message, "create refused");
                    outbox.lobby(Lobby::Error { message });
                    return;
                }
            };
            outbox.lobby(Lobby::RoomCreated { code: room.code.clone() });
            join(room, nick, device_token, conn_id, outbox, attached).await;
        }
        Msg::Lobby(Lobby::Join { nick, device_token, code }) => {
            if attached.is_some() {
                outbox.lobby(Lobby::Error { message: "already in a room".into() });
                return;
            }
            match hub.find(&code) {
                Ok(room) => join(room, nick, device_token, conn_id, outbox, attached).await,
                Err(message) => {
                    info!(conn = conn_id, code, %message, "join refused");
                    outbox.lobby(Lobby::Error { message });
                }
            }
        }
        Msg::Lobby(msg @ (Lobby::Ready | Lobby::Start | Lobby::Leave | Lobby::Kick { .. } | Lobby::Chat { .. })) => {
            match attached {
                Some(a) => {
                    let _ = a.room.commands.send(Command::Lobby { conn_id, msg }).await;
                }
                None => {
                    outbox.lobby(Lobby::Error { message: "not in a room".into() });
                }
            }
        }
        Msg::Lobby(_) | Msg::Snapshot(_) | Msg::Delta(_) | Msg::Welcome(_) => {
            outbox.lobby(Lobby::Error { message: "that message is the server's to send".into() });
        }
    }
}

/// Ask `room` for a seat and, given one, attach the connection to it.
async fn join(
    room: RoomHandle,
    nick: String,
    device_token: String,
    conn_id: u64,
    outbox: &Outbox,
    attached: &mut Option<Attached>,
) {
    let (reply, answer) = oneshot::channel();
    let link = ConnLink { id: conn_id, outbox: outbox.clone() };
    if room.commands.send(Command::Join { nick, device_token, conn: link, reply }).await.is_err() {
        outbox.lobby(Lobby::Error { message: format!("room {} is gone", room.code) });
        return;
    }
    match answer.await {
        Ok(Ok(joined)) => {
            *attached = Some(Attached { room, mailbox: joined.mailbox });
        }
        Ok(Err(message)) => {
            info!(conn = conn_id, code = room.code, %message, "seat refused");
            outbox.lobby(Lobby::Error { message });
        }
        Err(_) => {
            outbox.lobby(Lobby::Error { message: format!("room {} is gone", room.code) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_full_outbox_skips_snapshots_and_closes_on_a_reliable_send() {
        let (outbox, mut rx, close) = Outbox::pair();
        let bytes = Bytes::from_static(b"snap");
        for _ in 0..OUTBOX_DEPTH {
            assert_eq!(outbox.offer(bytes.clone()), Delivery::Sent);
        }
        assert_eq!(outbox.offer(bytes.clone()), Delivery::Skipped, "the slow client skips, nothing queues");
        assert_eq!(outbox.offer(bytes.clone()), Delivery::Skipped);
        assert!(rx.recv().await.is_some(), "draining one frees one slot");
        assert_eq!(outbox.offer(bytes.clone()), Delivery::Sent);
        assert_eq!(outbox.send(bytes.clone()), Delivery::Gone, "a reliable message on a full queue closes the client");
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), close.notified()).await.is_ok());
        drop(rx);
        assert_eq!(outbox.offer(bytes), Delivery::Gone);
        assert!(!outbox.is_open());
    }
}
