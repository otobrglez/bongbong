//! Bytes in, bytes out: one socket carries every message, so each is
//! framed as a one-byte kind tag followed by its body. Binary messages
//! (`Intent`, `Snapshot`, `Delta`, `Welcome`) are `postcard`; the lobby's
//! body is JSON (docs/online-coop-prd.md §4.3). The transport delivers
//! whole messages (a WebSocket frame each), so no length prefix is needed.

use std::fmt;

use crate::net::delta::SnapshotDelta;
use crate::net::wire::{IntentMsg, Lobby, Snapshot, Welcome};

/// The kind tags, the first byte of every message.
pub mod kind {
    /// `IntentMsg`, client to server.
    pub const INTENT: u8 = 1;
    /// A full `Snapshot`, server to client.
    pub const SNAPSHOT: u8 = 2;
    /// A `SnapshotDelta` against the previous snapshot, server to client.
    pub const DELTA: u8 = 3;
    /// `Welcome`, server to client.
    pub const WELCOME: u8 = 4;
    /// A `Lobby` message as JSON, either direction.
    pub const LOBBY: u8 = 5;
}

/// Any message the socket carries.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Intent(IntentMsg),
    Snapshot(Snapshot),
    Delta(SnapshotDelta),
    Welcome(Welcome),
    Lobby(Lobby),
}

impl Msg {
    /// The message's `kind` tag.
    pub fn kind(&self) -> u8 {
        match self {
            Msg::Intent(_) => kind::INTENT,
            Msg::Snapshot(_) => kind::SNAPSHOT,
            Msg::Delta(_) => kind::DELTA,
            Msg::Welcome(_) => kind::WELCOME,
            Msg::Lobby(_) => kind::LOBBY,
        }
    }
}

/// Why `decode` refused a message.
#[derive(Debug)]
pub enum DecodeError {
    /// No bytes at all, not even a kind tag.
    Empty,
    /// A kind tag this version does not know.
    UnknownKind(u8),
    /// The body did not parse as the kind's postcard type.
    Postcard(postcard::Error),
    /// The lobby body did not parse as JSON.
    Json(serde_json::Error),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Empty => write!(f, "empty message"),
            DecodeError::UnknownKind(k) => write!(f, "unknown message kind {k}"),
            DecodeError::Postcard(e) => write!(f, "malformed binary message: {e}"),
            DecodeError::Json(e) => write!(f, "malformed lobby message: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<postcard::Error> for DecodeError {
    fn from(e: postcard::Error) -> Self {
        DecodeError::Postcard(e)
    }
}

impl From<serde_json::Error> for DecodeError {
    fn from(e: serde_json::Error) -> Self {
        DecodeError::Json(e)
    }
}

/// The bytes of `msg`: its kind tag, then the body.
pub fn encode(msg: &Msg) -> Vec<u8> {
    let mut out = vec![msg.kind()];
    let body = match msg {
        Msg::Intent(m) => postcard::to_stdvec(m),
        Msg::Snapshot(m) => postcard::to_stdvec(m),
        Msg::Delta(m) => postcard::to_stdvec(m),
        Msg::Welcome(m) => postcard::to_stdvec(m),
        Msg::Lobby(m) => {
            out.extend(serde_json::to_vec(m).expect("a Lobby message always serialises"));
            return out;
        }
    };
    out.extend(body.expect("a wire struct always serialises"));
    out
}

/// The message in `bytes`, or why it is not one.
pub fn decode(bytes: &[u8]) -> Result<Msg, DecodeError> {
    let (&tag, body) = bytes.split_first().ok_or(DecodeError::Empty)?;
    Ok(match tag {
        kind::INTENT => Msg::Intent(postcard::from_bytes(body)?),
        kind::SNAPSHOT => Msg::Snapshot(postcard::from_bytes(body)?),
        kind::DELTA => Msg::Delta(postcard::from_bytes(body)?),
        kind::WELCOME => Msg::Welcome(postcard::from_bytes(body)?),
        kind::LOBBY => Msg::Lobby(serde_json::from_slice(body)?),
        other => return Err(DecodeError::UnknownKind(other)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::Mission;
    use crate::net::PROTOCOL_VERSION;
    use crate::net::delta::delta;
    use crate::net::events::WireEvent;
    use crate::net::wire::{RoundState, Seat, TankState, WeaponKind, WireOverrides};

    fn snapshot() -> Snapshot {
        Snapshot {
            tick: 900,
            server_ms: 15_000,
            tanks: vec![
                TankState { id: 0, x: 400, y: 800, vx: 50, vy: 0, dir: 3, hp: 100, weapon: WeaponKind::Laser, ammo: 3, ..Default::default() },
                TankState { id: 2, x: 2000, y: 1200, hp: 40, ..Default::default() },
            ],
            round: RoundState { wave: 2, alive: 1, pending: 3, intro: 0, outcome: Default::default() },
            events: vec![WireEvent::TankEntered { slot: 2 }],
            ..Default::default()
        }
    }

    #[test]
    fn every_kind_round_trips() {
        let mut next = snapshot();
        next.tick += 3;
        next.tanks[0].x += 40;
        let msgs = vec![
            Msg::Intent(IntentMsg { tick: 12, move_dir: 1, face: 0, fire: true }),
            Msg::Snapshot(snapshot()),
            Msg::Delta(delta(&snapshot(), &next)),
            Msg::Welcome(Welcome {
                protocol: PROTOCOL_VERSION,
                sim_version: "0.1.0".into(),
                seat: 1,
                roster: vec![Seat { seat: 0, nick: "host".into(), chassis: 4 }],
                map_toml: "size = [34, 17]\n".into(),
                seed: 0xB0B5,
                tuning_json: "{}".into(),
                overrides: WireOverrides { mission: Some(Mission::Hunt), ..Default::default() },
                enemy_count: Some(6),
                oil_cells: vec![40, 41],
                dead_cells: vec![77],
                snapshot: snapshot(),
            }),
            Msg::Lobby(Lobby::Chat { text: "gg".into() }),
        ];
        for msg in msgs {
            let bytes = encode(&msg);
            assert_eq!(bytes[0], msg.kind());
            assert_eq!(decode(&bytes).unwrap(), msg);
        }
    }

    #[test]
    fn kind_tags_are_distinct_and_stable() {
        assert_eq!([kind::INTENT, kind::SNAPSHOT, kind::DELTA, kind::WELCOME, kind::LOBBY], [1, 2, 3, 4, 5]);
    }

    #[test]
    fn intent_is_two_bytes_of_body_at_low_ticks() {
        let bytes = encode(&Msg::Intent(IntentMsg { tick: 5, move_dir: 4, face: 2, fire: false }));
        assert_eq!(bytes, vec![kind::INTENT, 5, 4, 2, 0]);
    }

    #[test]
    fn lobby_body_is_json() {
        let bytes = encode(&Msg::Lobby(Lobby::Kick { seat: 3 }));
        assert_eq!(&bytes[1..], br#"{"type":"kick","seat":3}"#);
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(matches!(decode(&[]), Err(DecodeError::Empty)));
        assert!(matches!(decode(&[9, 1, 2]), Err(DecodeError::UnknownKind(9))));
        assert!(matches!(decode(&[kind::SNAPSHOT, 0xff]), Err(DecodeError::Postcard(_))));
        assert!(matches!(decode(&[kind::LOBBY, b'{']), Err(DecodeError::Json(_))));
        let full = encode(&Msg::Snapshot(snapshot()));
        for cut in 1..full.len() {
            assert!(decode(&full[..cut]).is_err(), "a truncated snapshot at {cut} bytes must not decode");
        }
        let unknown_lobby = [&[kind::LOBBY][..], br#"{"type":"dance"}"#].concat();
        assert!(matches!(decode(&unknown_lobby), Err(DecodeError::Json(_))));
        assert!(!DecodeError::UnknownKind(9).to_string().is_empty());
    }
}
