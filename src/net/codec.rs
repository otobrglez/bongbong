//! Bytes in, bytes out: one socket carries every message, so each is
//! framed as a one-byte kind tag followed by its body. Binary messages
//! (`Intent`, `Snapshot`, `Delta`, `Welcome`) are `postcard`; the lobby's
//! body is JSON (docs/online-coop-prd.md §4.3). The transport delivers
//! whole messages (a WebSocket frame each), so no length prefix is needed.
//!
//! A `Welcome` is read from its leading `protocol` first: one from a room
//! on another `PROTOCOL_VERSION` is `Msg::ForeignWelcome` and the rest of
//! it is never parsed, since its layout is not this build's to read - so a
//! client learns why it cannot join instead of dropping a message it has no
//! words for (`net::client`).

use std::fmt;

use crate::net::PROTOCOL_VERSION;
use crate::net::delta::SnapshotDelta;
use crate::net::wire::{IntentMsg, Lobby, Ping, Pong, Snapshot, Welcome};

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
    /// A client's clock probe (`wire::Ping`).
    pub const PING: u8 = 6;
    /// Its echo (`wire::Pong`).
    pub const PONG: u8 = 7;
}

/// Any message the socket carries.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Intent(IntentMsg),
    Snapshot(Snapshot),
    Delta(SnapshotDelta),
    Welcome(Welcome),
    /// A `Welcome` from a room on another protocol, read as far as this
    /// build can: its leading `protocol`, the one field every version's
    /// `Welcome` starts with. `decode` makes it; a room never means to
    /// send one, and encoded it is that leading field alone, which reads
    /// back as itself.
    ForeignWelcome { protocol: u16 },
    Lobby(Lobby),
    Ping(Ping),
    Pong(Pong),
}

impl Msg {
    /// The message's `kind` tag.
    pub fn kind(&self) -> u8 {
        match self {
            Msg::Intent(_) => kind::INTENT,
            Msg::Snapshot(_) => kind::SNAPSHOT,
            Msg::Delta(_) => kind::DELTA,
            Msg::Welcome(_) | Msg::ForeignWelcome { .. } => kind::WELCOME,
            Msg::Lobby(_) => kind::LOBBY,
            Msg::Ping(_) => kind::PING,
            Msg::Pong(_) => kind::PONG,
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
        Msg::ForeignWelcome { protocol } => postcard::to_stdvec(protocol),
        Msg::Ping(m) => postcard::to_stdvec(m),
        Msg::Pong(m) => postcard::to_stdvec(m),
        Msg::Lobby(m) => {
            out.extend(serde_json::to_vec(m).expect("a Lobby message always serialises"));
            return out;
        }
    };
    out.extend(body.expect("a wire struct always serialises"));
    out
}

/// The message in `bytes`, or why it is not one. A `Welcome` whose leading
/// `protocol` is not this build's is `Msg::ForeignWelcome`, the rest of it
/// unread.
pub fn decode(bytes: &[u8]) -> Result<Msg, DecodeError> {
    let (&tag, body) = bytes.split_first().ok_or(DecodeError::Empty)?;
    Ok(match tag {
        kind::INTENT => Msg::Intent(postcard::from_bytes(body)?),
        kind::SNAPSHOT => Msg::Snapshot(postcard::from_bytes(body)?),
        kind::DELTA => Msg::Delta(postcard::from_bytes(body)?),
        kind::WELCOME => match postcard::take_from_bytes::<u16>(body)? {
            (protocol, _) if protocol != PROTOCOL_VERSION => Msg::ForeignWelcome { protocol },
            _ => Msg::Welcome(postcard::from_bytes(body)?),
        },
        kind::LOBBY => Msg::Lobby(serde_json::from_slice(body)?),
        kind::PING => Msg::Ping(postcard::from_bytes(body)?),
        kind::PONG => Msg::Pong(postcard::from_bytes(body)?),
        other => return Err(DecodeError::UnknownKind(other)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::Mission;
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
            round: RoundState { wave: 2, alive: 1, pending: 3, intro: 0, next_wave: 0, restart: 0, outcome: Default::default(), lamps_out: 0 },
            events: vec![WireEvent::TankEntered { slot: 2 }],
            ..Default::default()
        }
    }

    fn welcome() -> Welcome {
        Welcome {
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
            oil_prints: vec![crate::net::wire::OilPrintState { cell: 41, dir: 3, fade: 8 }],
            dead_cells: vec![77],
            trampled_cells: vec![300],
            burnt_grass: vec![299, 301],
            snapshot: snapshot(),
        }
    }

    #[test]
    fn every_kind_round_trips() {
        let mut next = snapshot();
        next.tick += 3;
        next.tanks[0].x += 40;
        let msgs = vec![
            Msg::Intent(IntentMsg { tick: 12, move_dir: 1, face: 0, fire: true, ..IntentMsg::default() }),
            Msg::Snapshot(snapshot()),
            Msg::Delta(delta(&snapshot(), &next)),
            Msg::Welcome(welcome()),
            Msg::Lobby(Lobby::Chat { text: "gg".into() }),
        ];
        for msg in msgs {
            let bytes = encode(&msg);
            assert_eq!(bytes[0], msg.kind());
            assert_eq!(decode(&bytes).unwrap(), msg);
        }
    }

    /// **A welcome from another protocol is read for its number alone.**
    /// Its body is another build's layout - here a field this build does
    /// not have, after the number - so nothing past the number is parsed,
    /// whether or not the rest would happen to decode; this build's own
    /// number with a body that does not parse is malformed, as before.
    #[test]
    fn a_welcome_from_another_protocol_is_read_for_its_number_alone() {
        for other in [33u16, PROTOCOL_VERSION + 1, 300] {
            let mut bytes = vec![kind::WELCOME];
            bytes.extend(postcard::to_stdvec(&other).unwrap());
            bytes.extend([0xff, 0x07, 0x42]);
            assert_eq!(decode(&bytes).unwrap(), Msg::ForeignWelcome { protocol: other });
            let alone = encode(&Msg::ForeignWelcome { protocol: other });
            assert_eq!(decode(&alone).unwrap(), Msg::ForeignWelcome { protocol: other }, "encoded, it reads back as itself");
        }
        // This build's welcome with another protocol's number in front: still foreign.
        let Msg::Welcome(_) = decode(&encode(&Msg::Welcome(welcome()))).unwrap() else { panic!("this build's welcome") };
        let mut theirs = welcome();
        theirs.protocol = 33;
        assert_eq!(decode(&encode(&Msg::Welcome(theirs))).unwrap(), Msg::ForeignWelcome { protocol: 33 });
        let mut cut = vec![kind::WELCOME];
        cut.extend(postcard::to_stdvec(&PROTOCOL_VERSION).unwrap());
        assert!(matches!(decode(&cut), Err(DecodeError::Postcard(_))), "this build's number and no body is malformed");
        assert!(matches!(decode(&[kind::WELCOME]), Err(DecodeError::Postcard(_))), "no number at all is malformed");
    }

    #[test]
    fn kind_tags_are_distinct_and_stable() {
        assert_eq!([kind::INTENT, kind::SNAPSHOT, kind::DELTA, kind::WELCOME, kind::LOBBY], [1, 2, 3, 4, 5]);
    }

    #[test]
    fn intent_is_two_bytes_of_body_at_low_ticks() {
        let bytes = encode(&Msg::Intent(IntentMsg { tick: 5, move_dir: 4, face: 2, fire: false, ..IntentMsg::default() }));
        // tick, move_dir, face, fire, lamp, then the pose an unowned
        // packet leaves at zero - owned, x, y, dir, vx, vy - the view
        // tick and fraction before the first snapshot, and no rod reticle.
        assert_eq!(bytes, vec![kind::INTENT, 5, 4, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
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
